use afn::{
    afn_voting_manager::VotingMode,
    config::{load_config_files, OffchainConfig},
    curse_beacon::CurseId,
    encryption::EncryptedSecrets,
    key_types::{BlessCurseKeys, BlessCurseKeysByChain},
    state,
    state::MonitorResult,
    worker,
};
use anyhow::{anyhow, bail, Context, Result};
use argh::FromArgs;
use std::{env, fs, io, os::unix::fs::OpenOptionsExt, path::Path, str::FromStr, sync::Arc};
use tracing::{error, warn};

const PASSPHRASE_ENV_VAR: &str = "AFN_PASSPHRASE";

fn user_passphrase() -> Result<String> {
    env::var(PASSPHRASE_ENV_VAR).with_context(|| {
        anyhow!("need to set a passphrase with the {PASSPHRASE_ENV_VAR} env variable")
    })
}

pub trait ExecutableCmd {
    fn execute_cmd(&self) -> Result<()>;
}

/// Ensure that we have entries in `keys` for each chain in `config`.
fn ensure_keys_for_chains(config: &OffchainConfig, keys: &BlessCurseKeysByChain) -> bool {
    for chain in config.all_chains() {
        if keys.get(chain).is_none() {
            error!("no key found for {chain}");
            return false;
        }
    }
    true
}

fn run_afn(
    voting_mode: VotingMode,
    keystore_path: Option<String>,
    manual_curse: Option<CurseId>,
) -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = load_config_files()?;

    let keys = match voting_mode {
        VotingMode::Active | VotingMode::DryRun => {
            let path = keystore_path
                .ok_or_else(|| anyhow!("keystore path is required in active & dryrun modes"))?;
            let keys: BlessCurseKeysByChain = {
                let file = fs::File::open(path)?;
                BlessCurseKeysByChain::from_reader(file, &user_passphrase()?)?
            };

            if !ensure_keys_for_chains(&config, &keys) {
                bail!("missing keys, update your keystore");
            }

            keys
        }
        VotingMode::Passive => BlessCurseKeysByChain::generate(&config.all_chains()),
    };

    loop {
        let ctx = worker::Context::new();
        let mut state = state::State::new_and_spawn_workers(
            Arc::clone(&ctx),
            config.clone(),
            keys.clone(),
            voting_mode,
            manual_curse,
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
    #[argh(option, description = "voting mode")]
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
}
impl ExecutableCmd for Run {
    fn execute_cmd(&self) -> Result<()> {
        run_afn(self.mode, self.keystore.clone(), self.manual_curse)
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
        let supported_chains = load_config_files()?.all_chains();
        eprintln!("generating keys for chains: {:?}", supported_chains);
        let bless_curse_keys_by_chain = BlessCurseKeysByChain::generate(&supported_chains);
        for chain_name in supported_chains {
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
        let supported_chains = load_config_files()?.all_chains();
        for &chain_name in supported_chains.iter() {
            if bless_curse_keys_by_chain.get(chain_name).is_none() {
                let secret_keys = BlessCurseKeys::generate();
                eprintln!("generated new keys for {}: {:?}", chain_name, secret_keys,);
                bless_curse_keys_by_chain.insert(chain_name, secret_keys);
            }
        }
        for chain_name in bless_curse_keys_by_chain.chains() {
            if !supported_chains.contains(&chain_name) {
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

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum KeystoreCmd {
    Gen(GenKeystore),
    Update(UpdateKeystore),
    Addrs(AddrsKeystore),
}
impl ExecutableCmd for KeystoreCmd {
    fn execute_cmd(&self) -> Result<()> {
        match self {
            Self::Gen(gen) => gen.execute_cmd(),
            Self::Update(update) => update.execute_cmd(),
            Self::Addrs(addrs) => addrs.execute_cmd(),
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
