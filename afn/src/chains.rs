use crate::generated_chains::LegacyChainName;

pub type ChainSelector = crate::generated_chains::ChainSelector;
pub type ChainName = LegacyChainName;

pub fn chain_selector(chain_name: ChainName) -> ChainSelector {
    match chain_name.try_into() {
        Ok(g_chain_name) => crate::generated_chains::chain_selector(g_chain_name),
        Err(_) => 0,
    }
}
