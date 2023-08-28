use std::time::{Duration, SystemTime};

use minieth::bytes::Address;
use rand::{seq::SliceRandom, SeedableRng};
use rand_chacha::ChaChaRng;

use crate::afn_voting_manager::VotingMode;

pub const DELTA_STAGE: Duration = Duration::from_secs(2 * 60);

fn permute<T: Copy>(things: &[T], seed: u64) -> Vec<T> {
    let mut rng = ChaChaRng::seed_from_u64(seed);
    let mut v: Vec<T> = things.to_vec();
    v.shuffle(&mut rng);
    v
}

fn current_seed() -> u64 {
    let st = SystemTime::now();
    let seconds = st.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    let minutes = seconds / 60;
    let hours = minutes / 60;
    hours
}

fn delay_with_seed(
    bless_vote_addr: Address,
    config: &crate::afn_contract::OnchainConfig,
    seed: u64,
) -> Option<Duration> {
    let permuted_voters = permute(&config.voters, seed);
    let mut stage = 0;
    let mut accumulated_weight = 0u16;
    for voter in permuted_voters {
        if voter.bless_vote_addr == bless_vote_addr {
            return Some(stage * DELTA_STAGE);
        }
        accumulated_weight += voter.bless_weight as u16;
        if (stage == 0 && accumulated_weight >= config.bless_weight_threshold) || stage > 0 {
            stage += 1;
        }
    }
    None
}

pub fn delay(
    bless_vote_addr: Address,
    config: &crate::afn_contract::OnchainConfig,
    mode: VotingMode,
) -> Option<Duration> {
    match mode {
        VotingMode::Active | VotingMode::DryRun => {
            delay_with_seed(bless_vote_addr, config, current_seed())
        }
        VotingMode::Passive => Some(Duration::ZERO),
    }
}
