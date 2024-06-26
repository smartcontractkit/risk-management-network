use afn::{
    afn_voting_manager::VotingMode,
    config::{load_config_files, OffchainConfig},
    curse_beacon::CurseId,
    encryption::EncryptedSecrets,
    forensics::LogRotate,
    key_types::{BlessCurseKeys, BlessCurseKeysByChain},
    state,
    state::MonitorResult,
    worker,
};
use anyhow::{anyhow, bail, Context, Result};
use argh::FromArgs;
use minieth::{
    bytes::{Address, Bytes},
    u256::U256,
};
use std::{
    env::{self, VarError},
    fs::{self},
    io::{self},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};
use tracing::{error, warn};

const LOG_JSON_ENV_VAR: &str = "AFN_LOG_JSON";
const PASSPHRASE_ENV_VAR: &str = "AFN_PASSPHRASE";
const METRICS_FILE_PATH_ENV_VAR: &str = "AFN_METRICS_FILE";
const CURSE_FILE_ENV_VAR: &str = "AFN_CURSE_FILE";

fn user_passphrase() -> Result<String> {
    env::var(PASSPHRASE_ENV_VAR).with_context(|| {
        anyhow!("need to set a passphrase with the {PASSPHRASE_ENV_VAR} env variable")
    })
}

fn metrics_file_path() -> Result<Option<PathBuf>> {
    match env::var(METRICS_FILE_PATH_ENV_VAR).as_deref() {
        Ok(path_str) => Ok(Some(PathBuf::from_str(path_str)?)),
        Err(VarError::NotPresent) => Ok(None),
        v => {
            bail!("invalid value for {METRICS_FILE_PATH_ENV_VAR}: {:?}", v);
        }
    }
}

fn curse_file_path() -> Result<Option<PathBuf>> {
    match env::var(CURSE_FILE_ENV_VAR).as_deref() {
        Ok(path_str) => Ok(Some(PathBuf::from_str(path_str)?)),
        Err(VarError::NotPresent) => Ok(None),
        v => {
            bail!("invalid value for {CURSE_FILE_ENV_VAR}: {:?}", v);
        }
    }
}

pub trait ExecutableCmd {
    fn execute_cmd(&self) -> Result<()>;
}

/// Ensure that we have entries in `keys` for each chain in `config`.
fn ensure_keys_for_chains(config: &OffchainConfig, keys: &BlessCurseKeysByChain) -> bool {
    for chain in config.enabled_chains() {
        if keys.get(chain).is_none() {
            error!("no key found for {chain}");
            return false;
        }
    }
    true
}

fn setup_tracing() -> Result<()> {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{filter, filter::LevelFilter, EnvFilter};

    let forensics_layer = tracing_subscriber::fmt::layer()
        .with_writer(Mutex::new(LogRotate::new(
            afn::config::FORENSICS_LOGROTATE_CONFIG,
        )?))
        .json()
        .flatten_event(true)
        .with_filter(filter::Targets::new().with_targets(vec![
            (
                minieth::json_rpc::LOG_SPAM_FORENSICS_TARGET,
                LevelFilter::TRACE,
            ),
            ("", LevelFilter::DEBUG),
        ]));

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let wrap_tracing_error = |e| anyhow::anyhow!("failed to initialize tracing: {e:?}");

    match env::var(LOG_JSON_ENV_VAR).as_deref() {
        Ok("on") => {
            let stdout_json_layer = tracing_subscriber::fmt::layer().json().flatten_event(true);
            tracing_subscriber::registry()
                .with(forensics_layer)
                .with(stdout_json_layer.with_filter(env_filter))
                .try_init()
                .map_err(wrap_tracing_error)?;
        }
        Ok("off") | Err(VarError::NotPresent) => {
            let stdout_compact_layer = tracing_subscriber::fmt::layer().compact();
            tracing_subscriber::registry()
                .with(forensics_layer)
                .with(stdout_compact_layer.with_filter(env_filter))
                .try_init()
                .map_err(wrap_tracing_error)?;
        }
        v => {
            bail!("invalid value for {LOG_JSON_ENV_VAR}: {:?}", v);
        }
    }

    Ok(())
}

fn run_afn(
    voting_mode: VotingMode,
    keystore_path: Option<String>,
    manual_curse: Option<CurseId>,
    dangerous_allow_multiple_rpcs: bool,
    check_rpc_pruning: bool,
    experimental: bool,
) -> Result<()> {
    setup_tracing()?;

    let config = load_config_files()?;

    let keys = match voting_mode {
        VotingMode::Active | VotingMode::UnreliableRemote | VotingMode::DryRun => {
            let path = keystore_path
                .ok_or_else(|| anyhow!("keystore path is required in {voting_mode:?} mode"))?;
            let keys: BlessCurseKeysByChain = {
                let file = fs::File::open(path)?;
                BlessCurseKeysByChain::from_reader(file, &user_passphrase()?)?
            };

            if !ensure_keys_for_chains(&config, &keys) {
                bail!("missing keys, update your keystore");
            }

            keys
        }
        VotingMode::Passive => BlessCurseKeysByChain::generate(&config.enabled_chains()),
    };

    loop {
        let ctx = worker::Context::new();
        let mut state = state::State::new_and_spawn_workers(
            Arc::clone(&ctx),
            config.clone(),
            keys.clone(),
            voting_mode,
            manual_curse,
            metrics_file_path()?,
            dangerous_allow_multiple_rpcs,
            check_rpc_pruning,
            curse_file_path()?,
            experimental,
        )?;
        let exit_reason = state.monitor()?;
        ctx.cancel();
        match exit_reason {
            MonitorResult::WorkersDied(dead_workers) => {
                error!("some worker died unexpectedly, shutdown all workers");
                bail!("workers died: {:?}", dead_workers);
            }
            MonitorResult::OnchainConfigChanged => {
                warn!("onchain config changed, shutdown all workers, and restart");
                warn!("restarting node");
            }
        }
    }
}

fn parse_manual_curse_id(s: &str) -> Result<CurseId, String> {
    match CurseId::from_str(s) {
        Ok(curse_id) => Ok(curse_id),
        Err(e) => Err(format!("Failed to parse curse id {s:?} due to error: {e}\n\n\
        NOTE: the curse id should be a 32-byte hex-encoded value such as 0x________________________________________________________________\n\
        We ask you to choose a value so as to avoid accidental duplicate curses where this command is run multiple times by accident. \
        If you're not sure how to set it, just pick a new random value for each curseable incident.")),
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "run")]
/// run afn!
struct Run {
    #[argh(
        option,
        description = "voting mode, can be (active|dryrun|passive). \
        active: actively monitors CCIP, sends vote-to-bless and vote-to-curse transactions; \
        dryrun: passively monitors CCIP while verifying the RMN contracts are correctly configured onchain with the addresses found in the supplied keystore, does not send transactions; \
        passive: passively monitors CCIP, without sending bless or curse votes or requiring a keystore
        "
    )]
    mode: VotingMode,
    #[argh(option, description = "path to keystore file")]
    keystore: Option<String>,
    #[argh(
        option,
        arg_name = "curse-id",
        description = "NOTE: the curse id should be a 32-byte hex-encoded value such as 0x________________________________________________________________ \
        We ask you to choose a value so as to avoid accidental duplicate curses where this command is run multiple times by accident. \
        If you're not sure how to set it, just pick a new random value for each curseable incident.",
        from_str_fn(parse_manual_curse_id)
    )]
    manual_curse: Option<CurseId>,
    #[argh(
        switch,
        description = "allow more than 1 RPC endpoints configured per chain. \
        We don't recommend having the RMN node query multiple RPC endpoints simultaneously for any chain as different RPC nodes can have conflicting chain states, causing false votes to curse. \
        It is challenging to distinguish between discrepancies from multiple RPC endpoints and actual cursable anomalies."
    )]
    dangerous_allow_multiple_rpcs: bool,
    #[argh(
        switch,
        description = "check on startup and on any onchain config change that rpcs have not pruned required blocks or logs. \
        This flag can be especially useful if you just performed changes to any of your full nodes. \
        The RMN node will fail to start if any required blocks or logs are suspected to be pruned. \
        In case of failure it is imperative that you check your full nodes for pruning, fix the issue and rerun using this flag to verify your fix."
    )]
    check_rpc_pruning: bool,
    #[argh(switch, hidden_help = true)]
    experimental: bool,
}
impl ExecutableCmd for Run {
    fn execute_cmd(&self) -> Result<()> {
        run_afn(
            self.mode,
            self.keystore.clone(),
            self.manual_curse,
            self.dangerous_allow_multiple_rpcs,
            self.check_rpc_pruning,
            self.experimental,
        )
    }
}

/// https://doc.rust-lang.org/std/fs/struct.File.html#method.create_new
fn file_create_new<P: AsRef<Path>>(path: P) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn write_encrypted_secrets<P: AsRef<Path>>(
    out_path: P,
    enc_secrets: &EncryptedSecrets,
) -> Result<()> {
    let out_file = file_create_new(out_path)?;
    serde_json::to_writer_pretty(out_file, &enc_secrets)?;
    Ok(())
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "gen")]
/// Generate keystore file for chains in the current config
struct GenKeystore {
    /// output file of generated keystore
    #[argh(option, short = 'o')]
    out: String,
}
impl ExecutableCmd for GenKeystore {
    fn execute_cmd(&self) -> Result<()> {
        let enabled_chains = load_config_files()?.enabled_chains();
        eprintln!("generating keys for chains: {:?}", enabled_chains);
        let bless_curse_keys_by_chain = BlessCurseKeysByChain::generate(&enabled_chains);
        for chain_name in enabled_chains {
            let secret_keys = bless_curse_keys_by_chain
                .get(chain_name)
                .ok_or_else(|| anyhow!("chain not found"))?;
            eprintln!("generated new keys for {}: {:?}", chain_name, secret_keys,);
        }
        let encrypted = bless_curse_keys_by_chain.encrypt(&user_passphrase()?)?;
        eprintln!("writing keystore to {}", self.out);
        write_encrypted_secrets(&self.out, &encrypted)?;
        Ok(())
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "update")]
/// Update keystore file for chains in the current config. Append new random keys for
/// chains that required by the current config, but not in the input keystore.
struct UpdateKeystore {
    /// input keystore file to update
    #[argh(positional)]
    file_to_update: String,
    /// output file of updated keystore
    #[argh(option, short = 'o')]
    out: String,
}
impl ExecutableCmd for UpdateKeystore {
    fn execute_cmd(&self) -> Result<()> {
        let mut bless_curse_keys_by_chain = {
            let file = fs::File::open(&self.file_to_update)?;
            let keystore_reader = io::BufReader::new(file);
            BlessCurseKeysByChain::from_reader(keystore_reader, &user_passphrase()?)?
        };
        let enabled_chains = load_config_files()?.enabled_chains();
        for &chain_name in enabled_chains.iter() {
            if bless_curse_keys_by_chain.get(chain_name).is_none() {
                let secret_keys = BlessCurseKeys::generate();
                eprintln!("generated new keys for {}: {:?}", chain_name, secret_keys,);
                bless_curse_keys_by_chain.insert(chain_name, secret_keys);
            }
        }
        for chain_name in bless_curse_keys_by_chain.chains() {
            if !enabled_chains.contains(&chain_name) {
                eprintln!(
                    "keystore contains keys for unsupported chain {}",
                    chain_name
                );
            }
        }
        let keystore = bless_curse_keys_by_chain.encrypt(&user_passphrase()?)?;
        eprintln!("writing updated keystore to {}", self.out);
        write_encrypted_secrets(&self.out, &keystore)?;
        Ok(())
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "addrs")]
/// Show voter addresses in the given keystore file
struct AddrsKeystore {
    /// keystore file to show addresses
    #[argh(positional)]
    file: String,
}
impl ExecutableCmd for AddrsKeystore {
    fn execute_cmd(&self) -> Result<()> {
        let bless_curse_keys = {
            let file = fs::File::open(&self.file)?;
            BlessCurseKeysByChain::from_reader(file, &user_passphrase()?)?
        };
        println!("{:#?}", bless_curse_keys);
        Ok(())
    }
}

fn address_from_hex_with_0x_prefix(s: &str) -> Result<Address, String> {
    if let Some(s_without_0x) = s.strip_prefix("0x") {
        match Address::from_str(s_without_0x) {
            Ok(v) => Ok(v),
            Err(e) => Err(e.to_string()),
        }
    } else {
        Err("missing 0x prefix".to_string())
    }
}
fn bytes_from_even_length_hex_with_0x_prefix(s: &str) -> Result<Bytes, String> {
    if let Some(s_without_0x) = s.strip_prefix("0x") {
        if s_without_0x.len() % 2 == 0 {
            match Bytes::from_str(s_without_0x) {
                Ok(v) => Ok(v),
                Err(e) => Err(e.to_string()),
            }
        } else {
            Err("invalid odd-length hex string".to_string())
        }
    } else {
        Err("missing 0x prefix".to_string())
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "manually-sign-transaction",
    description = "manually sign transaction with given parameters"
)]
struct SignManualTransaction {
    #[argh(option, description = "path to the keystore file to use")]
    keystore: String,

    #[argh(option, description = "decimal encoding, chain id of the transaction")]
    chain_id: u64,

    #[argh(option, description = "decimal encoding, nonce of the transaction")]
    nonce: u64,

    #[argh(option, description = "decimal encoding, gas price in wei")]
    gas_price_in_wei: u128,

    #[argh(option, description = "decimal encoding, gas limit")]
    gas: u128,

    #[argh(
        option,
        from_str_fn(address_from_hex_with_0x_prefix),
        description = "address of the sender that will be signing the transaction, this address must be included in the keystore"
    )]
    from: Address,

    #[argh(
        option,
        from_str_fn(address_from_hex_with_0x_prefix),
        description = "receiving address of the transaction"
    )]
    to: Address,

    #[argh(
        option,
        description = "decimal encoding, amount of native to transfer from sender to recipient "
    )]
    value_in_wei: u128,

    #[argh(
        option,
        from_str_fn(bytes_from_even_length_hex_with_0x_prefix),
        description = "hex encoding, the field data of the transaction"
    )]
    data_hex: Bytes,

    #[argh(
        switch,
        description = "required flag, please make sure you understand the risk of signing an arbirary manual transaction"
    )]
    i_really_know_what_i_am_doing: bool,
}

impl ExecutableCmd for SignManualTransaction {
    fn execute_cmd(&self) -> Result<()> {
        if !self.i_really_know_what_i_am_doing {
            bail!("Please provide flag '--i-really-know-what-i-am-doing' and make sure you understand the risk of signing an arbirary manual transaction")
        }

        let bless_curse_keys_by_chain = {
            let file = fs::File::open(&self.keystore)?;
            let keystore_reader = io::BufReader::new(file);
            BlessCurseKeysByChain::from_reader(keystore_reader, &user_passphrase()?)?
        };

        let mut key = None;
        for keys in bless_curse_keys_by_chain.0.values() {
            if keys.bless.address() == self.from {
                key = Some(keys.bless);
                break;
            }
            if keys.curse.address() == self.from {
                key = Some(keys.curse);
                break;
            }
        }

        if let Some(key) = key {
            let tx_request = minieth::tx::LegacyTransactionRequest {
                chain_id: self.chain_id,
                nonce: U256::from(self.nonce),
                gas_price: U256::from(self.gas_price_in_wei),
                gas: U256::from(self.gas),
                to: self.to,
                value: U256::from(self.value_in_wei),
                data: self.data_hex.clone(),
            };
            eprintln!(
                "singing non-eip1559 transaction with key of {}: {:?}",
                self.from, tx_request
            );
            let tx_bytes = tx_request.sign(&key.local_signer()).to_rlp_bytes();
            eprintln!("signed transaction = {}", Bytes::from(tx_bytes).to_string());
            Ok(())
        } else {
            bail!(
                "no matching found for {} in keystore {}",
                self.from,
                self.keystore
            )
        }
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum KeystoreCmd {
    Gen(GenKeystore),
    Update(UpdateKeystore),
    Addrs(AddrsKeystore),
    SignManualTransaction(SignManualTransaction),
}
impl ExecutableCmd for KeystoreCmd {
    fn execute_cmd(&self) -> Result<()> {
        match self {
            Self::Gen(gen) => gen.execute_cmd(),
            Self::Update(update) => update.execute_cmd(),
            Self::Addrs(addrs) => addrs.execute_cmd(),
            Self::SignManualTransaction(sign_manual_tx) => sign_manual_tx.execute_cmd(),
        }
    }
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "keystore")]
/// Keystore helper commands
struct KeystoreSub {
    #[argh(subcommand)]
    nested: KeystoreCmd,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "version")]
/// Returns git version
struct Version {}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum Subcommand {
    Run(Run),
    Keystore(KeystoreSub),
    Version(Version),
}

#[derive(FromArgs, PartialEq, Debug)]
/// afn
struct TopLevel {
    #[argh(subcommand)]
    nested: Subcommand,
}

fn main() -> Result<()> {
    let env: TopLevel = argh::from_env();

    match env.nested {
        Subcommand::Run(run_cmd) => match run_cmd.execute_cmd() {
            Ok(()) => Ok(()),
            Err(err) => {
                error!(?err, "got error, node stopped");
                Err(err)
            }
        },
        Subcommand::Keystore(KeystoreSub {
            nested: keystore_cmd,
        }) => keystore_cmd.execute_cmd(),
        Subcommand::Version(_) => {
            println!("{}", env!("GIT_HASH"));
            Ok(())
        }
    }
}
