use anyhow::Context;
use minieth::keccak::keccak256;
use rusqlite::{named_params, OptionalExtension};

use std::{
    cell::RefCell,
    cmp::{max, min},
    fs,
    path::Path,
    sync::Arc,
    time::Instant,
};

use minieth::{
    bytes::{Address, Bytes32},
    rpc::{EVMLog, Rpc, RpcError},
};

use crate::{common::ChainName, config::ChainStability};

const CACHE_BASE_DIR: &str = "./cache";

const SCHEMA_VERSION: u64 = 4;

pub struct CachedRpc {
    parent_worker_name: String,
    rpc: Arc<Rpc>,
    db: RefCell<LogsDb>,

    addresses: Vec<Address>,
    first_topics: Vec<Bytes32>,
}

fn query_shape_digest(
    chain: ChainName,
    chain_stability: ChainStability,
    addresses: &[Address],
    first_topics: &[Bytes32],
) -> anyhow::Result<Bytes32> {
    let mut addrs_buf = Vec::with_capacity(std::mem::size_of_val(addresses));
    for addr in addresses {
        addrs_buf.extend_from_slice(addr);
    }
    let mut first_topics_buf = Vec::with_capacity(std::mem::size_of_val(first_topics));
    for first_topic in first_topics {
        first_topics_buf.extend_from_slice(first_topic);
    }
    let mut final_buf =
        Vec::with_capacity(std::mem::size_of::<u64>() + 3 * std::mem::size_of::<Bytes32>());
    final_buf.extend_from_slice(&chain.chain_id().to_be_bytes());
    final_buf.extend_from_slice(&keccak256(&serde_json::to_vec(&chain_stability)?));
    final_buf.extend_from_slice(&keccak256(&addrs_buf));
    final_buf.extend_from_slice(&keccak256(&first_topics_buf));
    Ok(keccak256(&final_buf).into())
}

struct LogsDb(rusqlite::Connection);

const MAX_NUM_LOGS: u64 = 1_500;

#[derive(Debug)]
pub enum LogsReadError {
    TooManyLogs,
    NotAvailable,
    RusqliteError(rusqlite::Error),
    RpcError(RpcError),
}

impl From<rusqlite::Error> for LogsReadError {
    fn from(value: rusqlite::Error) -> Self {
        Self::RusqliteError(value)
    }
}

impl From<RpcError> for LogsReadError {
    fn from(value: RpcError) -> Self {
        Self::RpcError(value)
    }
}

impl LogsDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "
        create table if not exists logs (
            block_number int not null,
            block_hash blob not null,
            address blob not null,
            log_index int not null,
            topic0 blob not null, topic1 blob, topic2 blob, topic3 blob,
            data blob not null,
            unique(block_number, log_index)
        );

        create table if not exists queriable_block_ranges (from_block int not null, to_block int not null);
        create index if not exists queriable_block_ranges_covering_from_to on queriable_block_ranges(from_block, to_block);
        ",
        )?;
        Ok(Self(conn))
    }

    fn get_logs(&mut self, from_block: u64, to_block: u64) -> Result<Vec<EVMLog>, LogsReadError> {
        let tx = self.0.transaction()?;
        let db_to_block: Option<u64> = tx
            .query_row(
                "select to_block from queriable_block_ranges where from_block <= :from_block and to_block >= :from_block order by to_block desc limit 1",
                named_params! {
                    ":from_block": from_block,
                },
                |row| row.get(0),
            )
            .optional()?;
        match db_to_block {
            Some(db_to_block) => {
                if db_to_block >= to_block {
                } else {
                    return Err(LogsReadError::TooManyLogs);
                }
            }
            None => return Err(LogsReadError::NotAvailable),
        }

        let from_block_to_block_params = named_params! {
            ":from_block": from_block,
            ":to_block": to_block,
        };

        let num_logs: u64 = match tx.query_row(
            "select count(1) from logs
            where block_number >= :from_block and block_number <= :to_block",
            from_block_to_block_params,
            |row| row.get(0),
        ) {
            Ok(num_logs) => num_logs,
            Err(e) => return Err(e.into()),
        };

        if num_logs > MAX_NUM_LOGS {
            return Err(LogsReadError::TooManyLogs);
        }

        let mut st = tx.prepare_cached(
            "select
                block_number, block_hash, address, log_index, topic0, topic1, topic2, topic3, data
            from logs
            where block_number >= :from_block and block_number <= :to_block
            order by block_number, log_index asc",
        )?;
        let log_results = st.query_map(from_block_to_block_params, evm_log_from_row)?;
        Ok(log_results.collect::<Result<Vec<EVMLog>, rusqlite::Error>>()?)
    }

    pub fn save_logs(
        &mut self,
        logs: &[EVMLog],
        from_block: u64,
        to_block: u64,
    ) -> Result<(), rusqlite::Error> {
        let tx = self.0.transaction()?;
        {
            let min_from_block = tx
                .query_row(
                    "select from_block from queriable_block_ranges
                    where to_block+1 >= :from_block_new order by from_block asc limit 1",
                    named_params! { ":from_block_new": from_block, },
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(from_block);

            let max_to_block = tx
                .query_row(
                    "select to_block from queriable_block_ranges
                    where from_block-1 <= :to_block_new order by to_block desc limit 1",
                    named_params! { ":to_block_new": to_block, },
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(to_block);

            tx.execute("delete from queriable_block_ranges where from_block >= :min_from_block and to_block <= :max_to_block", named_params! {
                ":min_from_block": min_from_block,
                ":max_to_block": max_to_block,
            })?;
            tx.execute("insert into queriable_block_ranges (from_block, to_block) values (:min_from_block, :max_to_block)", named_params! {
                ":min_from_block": min(from_block, min_from_block),
                ":max_to_block": max(to_block, max_to_block),
            })?;
        }
        {
            let mut st = tx
            .prepare_cached(
                "insert or replace into logs (
            block_number, block_hash, address, log_index, topic0, topic1, topic2, topic3, data
        ) values (
            :block_number, :block_hash, :address, :log_index, :topic0, :topic1, :topic2, :topic3, :data
        )",
            )?;
            for log in logs {
                st.execute(named_params! {
                    ":block_number": log.block_number,
                    ":block_hash": log.block_hash.as_slice(),
                    ":address": log.address.as_slice(),
                    ":log_index": log.log_index,
                    ":topic0": log.topics.get(0).map(Bytes32::as_slice),
                    ":topic1": log.topics.get(1).map(Bytes32::as_slice),
                    ":topic2": log.topics.get(2).map(Bytes32::as_slice),
                    ":topic3": log.topics.get(3).map(Bytes32::as_slice),
                    ":data": log.data.as_ref(),
                })?;
            }
        }
        tx.commit()
    }
}

fn evm_log_from_row(r: &rusqlite::Row) -> Result<EVMLog, rusqlite::Error> {
    let block_number = r.get("block_number")?;
    let log_index = r.get("log_index")?;
    let block_hash: [u8; 32] = r.get("block_hash")?;
    let address: [u8; 20] = r.get("address")?;
    let topic0: Option<[u8; 32]> = r.get("topic0")?;
    let topic1: Option<[u8; 32]> = r.get("topic1")?;
    let topic2: Option<[u8; 32]> = r.get("topic2")?;
    let topic3: Option<[u8; 32]> = r.get("topic3")?;
    let data: Vec<u8> = r.get("data")?;
    let all_topics = [topic0, topic1, topic2, topic3];
    let topics = all_topics
        .into_iter()
        .flatten()
        .map(Bytes32::from)
        .collect();
    Ok(EVMLog {
        log_index,
        address: address.into(),
        data: data.into(),
        topics,
        block_hash: block_hash.into(),
        block_number,
    })
}

fn str_to_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        })
        .collect()
}

impl CachedRpc {
    pub fn new(
        chain: ChainName,
        addresses: &[Address],
        first_topics: Vec<Bytes32>,
        rpc: Arc<Rpc>,
        chain_stability: ChainStability,
        parent_worker_name: &str,
    ) -> anyhow::Result<Self> {
        let cache_dir = Path::new(CACHE_BASE_DIR).join(format!("v{SCHEMA_VERSION}"));
        fs::create_dir_all(&cache_dir).context("failed to create cache directory")?;
        let db_filename = format!(
            "{}_{}",
            str_to_filename(parent_worker_name),
            query_shape_digest(chain, chain_stability, addresses, &first_topics)?,
        );
        let our_db_file = cache_dir.join(db_filename);
        tracing::debug!("{parent_worker_name}: db in {our_db_file:?}");
        let db = LogsDb::open(our_db_file).context("failed to open logs db")?;

        let addresses = addresses.to_vec();

        Ok(Self {
            parent_worker_name: parent_worker_name.to_owned(),
            rpc,
            db: db.into(),
            addresses,
            first_topics,
        })
    }

    pub fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        cache: bool,
    ) -> Result<Vec<EVMLog>, LogsReadError> {
        let start = Instant::now();
        if cache {
            match self.db.borrow_mut().get_logs(from_block, to_block) {
                Ok(cached_response) => {
                    tracing::debug!(
                        "cache hit for {} {from_block}..{to_block} :) (took: {:?}, len: {})",
                        self.parent_worker_name,
                        start.elapsed(),
                        cached_response.len(),
                    );
                    return Ok(cached_response);
                }
                Err(LogsReadError::TooManyLogs) => {
                    return Err(LogsReadError::TooManyLogs);
                }
                Err(LogsReadError::NotAvailable) => {}
                Err(LogsReadError::RusqliteError(e)) => {
                    tracing::warn!(
                        "failed to read from cache {} {from_block}..{to_block} (took {:?}, error {e:?})",
                        self.parent_worker_name,
                        start.elapsed(),
                    );
                }
                Err(LogsReadError::RpcError(_)) => unreachable!(),
            }
        }
        match self
            .rpc
            .get_logs(from_block, to_block, &self.addresses, &self.first_topics)
        {
            Ok(fresh_response) => {
                if cache {
                    let start = Instant::now();
                    match self
                        .db
                        .borrow_mut()
                        .save_logs(&fresh_response, from_block, to_block)
                    {
                        Ok(_) => {
                            tracing::debug!(
                                "wrote to cache {} {from_block}..{to_block} (took {:?})",
                                self.parent_worker_name,
                                start.elapsed(),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "failed to write to cache {} {from_block}..{to_block} (took {:?}, error {e:?})",
                                self.parent_worker_name,
                                start.elapsed(),
                            );
                        }
                    }
                }
                Ok(fresh_response)
            }
            Err(RpcError::NotFound) => Err(LogsReadError::NotAvailable),
            Err(err) => {
                tracing::warn!(
                    "got rpc error when querying logs for {} {from_block}..{to_block}, assuming it means TooManyLogs (took {:?}, error {err:?})",
                    self.parent_worker_name,
                    start.elapsed(),
                );
                Err(LogsReadError::TooManyLogs)
            }
        }
    }
}
