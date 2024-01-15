use crate::common::ChainName;

pub type ChainSelector = u64;

pub fn chain_selector(chain: ChainName) -> ChainSelector {
    match chain {
        ChainName::Arbitrum => 4949039107694359620,
        ChainName::Avax => 6433500567565415381,
        ChainName::Base => 15971525489660198786,
        ChainName::Bsc => 11344663589394136015,
        ChainName::Ethereum => 5009297550715157269,
        ChainName::Kroma => 3719320017875267166,
        ChainName::Optimism => 3734403246176062136,
        ChainName::Polygon => 4051577828743386545,
        ChainName::PolygonZkEvm => 4348158687435793198,
        ChainName::Scroll => 13204309965629103672,
        ChainName::Wemix => 5142893604156789321,

        ChainName::ArbitrumGoerli => 0x54ABF9FB1AFEAF95,
        ChainName::ArbitrumSepolia => 3478487238524512106,
        ChainName::AvaxFuji => 0xCCF0A31A221F3C9B,
        ChainName::BaseGoerli => 5790810961207155433,
        ChainName::BaseSepolia => 10344971235874465080,
        ChainName::BscTestnet => 13264668187771770619,
        ChainName::Goerli => unimplemented!(),
        ChainName::KromaSepolia => 5990477251245693094,
        ChainName::OptimismGoerli => 0x24F9B897EF58A922,
        ChainName::OptimismSepolia => 5224473277236331295,
        ChainName::PolygonMumbai => 0xADECC60412CE25A5,
        ChainName::PolygonZkEvmTestnet => 11059667695644972511,
        ChainName::ScrollSepolia => 2279865765895943307,
        ChainName::Sepolia => 0xDE41BA4FC9D91AD9,
        ChainName::WemixTestnet => 9284632837123596123,
        ChainName::ZkSyncGoerli => 6802309497652714138,
    }
}
