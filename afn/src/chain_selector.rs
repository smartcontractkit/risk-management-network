use crate::common::ChainName;

pub type ChainSelector = u64;

pub fn chain_selector(chain: ChainName) -> ChainSelector {
    match chain {
        ChainName::Ethereum => 5009297550715157269,
        ChainName::Base => 15971525489660198786,
        ChainName::Bsc => 11344663589394136015,
        ChainName::Arbitrum => 4949039107694359620,
        ChainName::Optimism => 3734403246176062136,
        ChainName::Avax => 6433500567565415381,
        ChainName::Polygon => 4051577828743386545,

        ChainName::Sepolia => 0xDE41BA4FC9D91AD9,
        ChainName::BaseGoerli => 5790810961207155433,
        ChainName::BscTestnet => 13264668187771770619,
        ChainName::OptimismGoerli => 0x24F9B897EF58A922,
        ChainName::ArbitrumGoerli => 0x54ABF9FB1AFEAF95,
        ChainName::AvaxFuji => 0xCCF0A31A221F3C9B,
        ChainName::PolygonMumbai => 0xADECC60412CE25A5,
        ChainName::Goerli => unimplemented!(),
    }
}
