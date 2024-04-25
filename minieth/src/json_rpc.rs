use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::min;
use std::fmt::Display;
use std::io::{self, Read};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use thiserror::Error;
use ureq::OrAnyStatus;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
pub struct JsonRpcResponseError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl Display for JsonRpcResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Success {
        jsonrpc: String,
        id: u64,
        result: Value,
    },
    Failure {
        jsonrpc: String,
        id: u64,
        error: JsonRpcResponseError,
    },
}

impl JsonRpcResponse {
    pub fn id(&self) -> u64 {
        match self {
            JsonRpcResponse::Success { id, .. } => *id,
            JsonRpcResponse::Failure { id, .. } => *id,
        }
    }

    pub fn jsonrpc(&self) -> &str {
        match self {
            JsonRpcResponse::Success { jsonrpc, .. } => jsonrpc,
            JsonRpcResponse::Failure { jsonrpc, .. } => jsonrpc,
        }
    }
}

impl From<JsonRpcResponse> for Result<Value, JsonRpcResponseError> {
    fn from(value: JsonRpcResponse) -> Self {
        match value {
            JsonRpcResponse::Success { result, .. } => Ok(result),
            JsonRpcResponse::Failure { error, .. } => Err(error),
        }
    }
}

#[derive(Error, Debug)]
pub enum NetError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

impl From<ureq::Transport> for NetError {
    fn from(value: ureq::Transport) -> Self {
        Self::Transport(format!("{:?}", value))
    }
}

#[derive(Error, Debug)]
pub enum JsonRpcError {
    #[error("net error: {0}")]
    NetError(#[from] NetError),
    #[error("http response code {0}: {1}")]
    HttpStatus(u32, String),
    #[error("serde json error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("jsonrpc response indicates an error: {0:?}")]
    ErrorResponse(JsonRpcResponseError),
    #[error("unexpected id in response (expected {expected}, actual {actual})")]
    UnexpectedId { expected: u64, actual: u64 },
    #[error("unexpected jsonrpc version: {0:?}")]
    UnexpectedVersion(String),
    #[error("not all requests in the batch were responded to")]
    IncompleteBatch,
}

#[derive(Error, Debug)]
pub enum UrlError {
    #[error("failed to parse url: {0}")]
    ParseError(#[from] url::ParseError),
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
}

pub trait JsonRpcCall {
    fn call(&self, calls: &[Call]) -> Result<Vec<CallResponse>, JsonRpcError>;
}

#[derive(Debug)]
pub struct JsonRpc {
    locked: Mutex<JsonRpcLocked>,
    agent: ureq::Agent,
    servers: Vec<ServerState>,
}

#[derive(Debug)]
pub struct JsonRpcLocked {
    server_id: usize,
    invocations_count: usize,
}

#[derive(Debug)]
struct ServerState {
    url: String,
    next_id: Mutex<u64>,
}

const JSONRPC_VERSION: &str = "2.0";

#[derive(Error, Debug)]
pub enum JsonRpcNewError {
    #[error("url error")]
    UrlError(#[from] UrlError),
    #[error("not enough urls")]
    NotEnoughUrls,
}

const SINGLE_REQUEST_TIMEOUT_CONNECT: Duration = Duration::from_secs(5);
const SINGLE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_IDLE_CONNECTIONS_PER_HOST: usize = 10;

impl JsonRpc {
    pub fn new_many(rpc_urls: &[String]) -> Result<Self, JsonRpcNewError> {
        if rpc_urls.is_empty() {
            return Err(JsonRpcNewError::NotEnoughUrls);
        }

        let servers = rpc_urls
            .iter()
            .map(|url| ServerState::new(url))
            .collect::<Result<_, _>>()?;

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(SINGLE_REQUEST_TIMEOUT_CONNECT)
            .timeout(SINGLE_REQUEST_TIMEOUT)
            .max_idle_connections_per_host(MAX_IDLE_CONNECTIONS_PER_HOST)
            .build();

        Ok(Self {
            agent,
            servers,
            locked: Mutex::new(JsonRpcLocked {
                server_id: 0,
                invocations_count: 0,
            }),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    pub method: String,
    pub params: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: Value,
    id: u64,
}

pub type CallResponse = Result<Value, JsonRpcResponseError>;

impl ServerState {
    fn new(url: &str) -> Result<Self, UrlError> {
        let parsed_url = Url::parse(url)?;
        let scheme = parsed_url.scheme();
        if !matches!(scheme, "http" | "https") {
            return Err(UrlError::UnsupportedScheme(scheme.to_string()));
        }

        Ok(ServerState {
            url: url.to_owned(),
            next_id: Mutex::new(0),
        })
    }

    fn new_base_id(&self, num_calls: usize) -> u64 {
        let mut next_id = self.next_id.lock().unwrap();
        let prev = *next_id;
        *next_id += num_calls as u64;
        prev
    }

    fn produce_payload_json(&self, calls: &[Call]) -> (u64, Value) {
        let base_id = self.new_base_id(calls.len());
        (
            base_id,
            json!(calls
                .iter()
                .enumerate()
                .map(|(i, call)| {
                    JsonRpcRequest {
                        jsonrpc: JSONRPC_VERSION.to_owned(),
                        method: call.method.clone(),
                        params: json!(call.params),
                        id: base_id + i as u64,
                    }
                })
                .collect::<Vec<_>>()),
        )
    }
}

const MIN_RETRIES: u64 = 2;
const MIN_WAIT: Duration = SINGLE_REQUEST_TIMEOUT;
const MIN_RETRY_WAIT: Duration = Duration::from_millis(250);

const MAX_RESPONSE_BYTES: usize = (1_500 * 55_000) * (10 + 10 + 1) / 10;

impl JsonRpc {
    fn send(&self, url: &str, payload: &[u8]) -> Result<(u32, Vec<u8>), NetError> {
        let response = self
            .agent
            .post(url)
            .set("Content-Type", "application/json")
            .send_bytes(payload)
            .or_any_status()?;
        let status = response.status();
        let mut buf = Vec::new();
        response
            .into_reader()
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut buf)?;
        if buf.len() > MAX_RESPONSE_BYTES {
            return Err(NetError::Io(io::Error::new(
                io::ErrorKind::Other,
                format!("received more than the expected {MAX_RESPONSE_BYTES} bytes"),
            )));
        }
        Ok((status.into(), buf))
    }
}

fn potentially_faulty_response(r: &JsonRpcResponse) -> bool {
    match r {
        JsonRpcResponse::Failure {
            jsonrpc: _,
            id: _,
            error: _,
        } => true,
        JsonRpcResponse::Success {
            jsonrpc: _,
            id: _,
            result,
        } => *result == Value::Null,
    }
}

fn count_potentially_faulty_responses(r: &[JsonRpcResponse]) -> usize {
    r.iter().filter(|r| potentially_faulty_response(r)).count()
}

fn cap_undecodable_response(s: &[u8]) -> String {
    String::from_utf8_lossy(&s[..min(s.len(), 1_000)]).to_string()
}

fn better_jsonrpc_batch_result(
    older: Result<Vec<JsonRpcResponse>, JsonRpcError>,
    newer: Result<Vec<JsonRpcResponse>, JsonRpcError>,
) -> Result<Vec<JsonRpcResponse>, JsonRpcError> {
    match (older, newer) {
        (Ok(older), Ok(newer)) => {
            if count_potentially_faulty_responses(&older)
                <= count_potentially_faulty_responses(&newer)
            {
                Ok(older)
            } else {
                Ok(newer)
            }
        }
        (Err(_), Ok(newer)) => Ok(newer),
        (Ok(older), Err(_)) => Ok(older),
        (Err(_), Err(newer)) => Err(newer),
    }
}

pub const LOG_SPAM_FORENSICS_TARGET: &str = "_rpc_forensics";

fn log_forensics(url: &str, sent: &Value, received: &Result<(u32, Vec<u8>), NetError>) {
    match &received {
        Ok((http_code, body)) => {
            tracing::trace!(
                target: LOG_SPAM_FORENSICS_TARGET,
                url,
                %sent,
                http_code,
                received = %String::from_utf8_lossy(body),
            );
        }
        Err(error) => {
            tracing::trace!(
                target: LOG_SPAM_FORENSICS_TARGET,
                url,
                %sent,
                error = %error,
            );
        }
    }
}

impl JsonRpcCall for JsonRpc {
    fn call(&self, calls: &[Call]) -> Result<Vec<CallResponse>, JsonRpcError> {
        let (invocations_count, mut server_id) = {
            let mut locked = self.locked.lock().unwrap();
            locked.invocations_count += 1;
            (locked.invocations_count, locked.server_id)
        };
        let initial_server_id = server_id;
        let result: Result<Vec<JsonRpcResponse>, JsonRpcError> = {
            let loop_start = Instant::now();
            let mut best_jsonrpc_batch_result: Result<Vec<JsonRpcResponse>, JsonRpcError> =
                Err(JsonRpcError::IncompleteBatch);
            let mut i = 1;
            loop {
                let server = &self.servers[server_id];
                let (base_id, payload) = server.produce_payload_json(calls);
                let encoded_payload = serde_json::to_vec(&payload)?;
                let start = Instant::now();
                let http_result = self.send(&server.url, &encoded_payload);
                log_forensics(&server.url, &payload, &http_result);

                let took = start.elapsed();
                let tried_all_servers = i as usize >= self.servers.len();
                let exhausted_retries =
                    tried_all_servers && i >= MIN_RETRIES && loop_start.elapsed() >= MIN_WAIT;
                let jsonrpc_batch_result = match http_result {
                    Ok((200, r)) => {
                        let maybe_responses: Result<Vec<JsonRpcResponse>, serde_json::Error> =
                            serde_json::from_slice(&r);
                        match maybe_responses {
                            Err(_) => {
                                Err(JsonRpcError::HttpStatus(200, cap_undecodable_response(&r)))
                            }
                            Ok(mut responses) => {
                                responses.sort_by_key(|response| response.id());
                                if !(base_id..base_id + calls.len() as u64)
                                    .eq(responses.iter().map(|r| r.id()))
                                {
                                    Err(JsonRpcError::IncompleteBatch)
                                } else if let Some(response) =
                                    responses.iter().find(|r| r.jsonrpc() != JSONRPC_VERSION)
                                {
                                    Err(JsonRpcError::UnexpectedVersion(
                                        response.jsonrpc().to_owned(),
                                    ))
                                } else {
                                    Ok(responses)
                                }
                            }
                        }
                    }
                    Ok((code, r)) => {
                        Err(JsonRpcError::HttpStatus(code, cap_undecodable_response(&r)))
                    }
                    Err(e) => Err(e.into()),
                };

                if i > 1
                    || jsonrpc_batch_result.is_err()
                    || jsonrpc_batch_result
                        .as_ref()
                        .map(|r| count_potentially_faulty_responses(r))
                        .unwrap_or(0)
                        > 0
                {
                    tracing::warn!("[{invocations_count}] request {i} to {} with base_id={base_id} payload='{payload}' returned {jsonrpc_batch_result:?}", server.url);
                }

                best_jsonrpc_batch_result =
                    better_jsonrpc_batch_result(best_jsonrpc_batch_result, jsonrpc_batch_result);

                if exhausted_retries {
                    break best_jsonrpc_batch_result;
                } else if let Ok(ref decoded_response) = best_jsonrpc_batch_result {
                    if count_potentially_faulty_responses(decoded_response) == 0
                        || tried_all_servers
                    {
                        break best_jsonrpc_batch_result;
                    }
                }
                i += 1;
                server_id = (server_id + 1) % self.servers.len();
                if took < MIN_RETRY_WAIT {
                    thread::sleep(MIN_RETRY_WAIT - took);
                }
            }
        };

        let responses = result?;
        if initial_server_id != server_id {
            let mut locked = self.locked.lock().unwrap();
            locked.server_id = server_id;
        }
        Ok(responses.into_iter().map(|r| r.into()).collect())
    }
}
