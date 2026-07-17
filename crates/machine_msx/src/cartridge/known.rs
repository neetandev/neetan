//! Verified cartridge mapper assignments.

use super::CartridgeMapper;

#[derive(Debug, Clone, Copy)]
struct KnownCartridge {
    name: &'static str,
    digest: &'static str,
    mapper: CartridgeMapper,
}

/// Cartridge mappings that cannot be distinguished safely by ROM heuristics.
const KNOWN_CARTRIDGES: &[KnownCartridge] = &[
    KnownCartridge {
        name: "Aliens - Alien 2",
        digest: "e09ab21eda82126ea0336d54e04b92a0c77cea4bc3c15d092e27b4e84d29ce2b",
        mapper: CartridgeMapper::Ascii16,
    },
    KnownCartridge {
        name: "Generic European MSX-DOS 2.20",
        digest: "083f965452828757c7859383a15ed224746190875d522229d41f5bd3cda3be98",
        mapper: CartridgeMapper::MsxDos2,
    },
    KnownCartridge {
        name: "Cross Blaim",
        digest: "2466bbb0db836f5f4ea9beb280928d17de5d658ce024d9ab3fe10c9068f9343d",
        mapper: CartridgeMapper::CrossBlaim,
    },
    KnownCartridge {
        name: "Konami's Game Master 2",
        digest: "2f3ee5235cc0078499d4be07cde2e5ea5ba654daa73ca1153cd153e21431e120",
        mapper: CartridgeMapper::GameMaster2,
    },
    KnownCartridge {
        name: "Konami's Game Master 2",
        digest: "a9104add7b9533c196860c121258dcf8903d9d957e72a9c4cbddd7c7b8270461",
        mapper: CartridgeMapper::GameMaster2,
    },
    KnownCartridge {
        name: "Harry Fox - Yuki no Maou Hen",
        digest: "ba36e24f68457b62d202a53c89f88e1d824121babe4e40646da7e80d9638eb15",
        mapper: CartridgeMapper::HarryFox,
    },
    KnownCartridge {
        name: "Konami's Synthesizer",
        digest: "f389a3d2a3dd57a684c131be234971429e331c9cee7ad9b7fd66d3fb92b14fa2",
        mapper: CartridgeMapper::Synthesizer,
    },
    KnownCartridge {
        name: "Playball",
        digest: "fb307ebd1d3cb406a01e72d230b10b551d56ec70bb191f945d7f1b32ba4c0557",
        mapper: CartridgeMapper::PlayBall,
    },
    KnownCartridge {
        name: "R-Type",
        digest: "9a0b3bf5e2fa52ee10cb35a13fa3a7cf99be09989926f3d2b13025e4cee4fbb6",
        mapper: CartridgeMapper::RType,
    },
    KnownCartridge {
        name: "R-Type",
        digest: "3aac152e527f7e01355cdbf29ad90786773476a66c8a14ef2b4a5837b35885ef",
        mapper: CartridgeMapper::RType,
    },
    KnownCartridge {
        name: "R-Type",
        digest: "e979c659c2b5b8d07c28002b1561cade9ed3778064b35bb92660f6449fc81115",
        mapper: CartridgeMapper::RType,
    },
    KnownCartridge {
        name: "Hai no Majutsushi - Mahjong 2",
        digest: "d1eac2d8702dc8acd487de039b27f10c1b91be519a240c853cdc551f562a7655",
        mapper: CartridgeMapper::Majutsushi,
    },
    KnownCartridge {
        name: "Halnote",
        digest: "563f2ce72d74936ef9190aa6cadda4741bed020755ff7e746c43e795e6928c6c",
        mapper: CartridgeMapper::Halnote,
    },
    KnownCartridge {
        name: "Moero!! Nettou Yakyuu '88",
        digest: "e5494b59e7b4ad277a88dc2222b859f3254d4551a61ca968122e986c540b2b95",
        mapper: CartridgeMapper::NettouYakyuu,
    },
    KnownCartridge {
        name: "Super Lode Runner",
        digest: "dd1e94cb1ac3cdba97c4893ca8091715201a882f3272623ef7fe5e1e300ed495",
        mapper: CartridgeMapper::SuperLodeRunner,
    },
    KnownCartridge {
        name: "Wizardry",
        digest: "db67a80e3b6c359bded13a70d47b146bed5595609ea5a7936c4baaf1d97489ce",
        mapper: CartridgeMapper::Wizardry,
    },
    KnownCartridge {
        name: "FM Pana Amusement Cartridge",
        digest: "7b767ab4ccb835d5f6e58c7d1514f1157d1f9680318e4155537e83553cd85eb2",
        mapper: CartridgeMapper::FmPac,
    },
    KnownCartridge {
        name: "Confused?",
        digest: "f8656edb69dea8d272e68dcc4de3ef846d7bab3691554d753d42322f15cbea52",
        mapper: CartridgeMapper::Generic8,
    },
    KnownCartridge {
        name: "Demonia I",
        digest: "84a7460a3beb7b175884a3d490f4ffebb526edb928ff3989e637071e21485fe2",
        mapper: CartridgeMapper::Generic8,
    },
    KnownCartridge {
        name: "Deep Dungeon 1",
        digest: "2e6075f77d909b265ef50a2bd395bcba82424aae0d17388ca5810f03100fd65d",
        mapper: CartridgeMapper::Ascii8Sram2,
    },
    KnownCartridge {
        name: "Deep Dungeon 2",
        digest: "49ff14419778c078f2981b30118d1511ba328ab26c4171dc814634e53351a127",
        mapper: CartridgeMapper::Ascii8Sram2,
    },
    KnownCartridge {
        name: "Dires",
        digest: "48a15128dcccc0927c110a4bb5367863c060bbacc7ee88f53c7753cc238fcc9d",
        mapper: CartridgeMapper::Ascii8Sram2,
    },
    KnownCartridge {
        name: "Elthlead",
        digest: "895954e4a5b143198792cd0f9b77bae7ba459584a49f2dd476c42f565a707883",
        mapper: CartridgeMapper::Ascii8Sram2,
    },
    KnownCartridge {
        name: "Kisei",
        digest: "5a74ada3a353ab32cd2d74b137999d59eb40b552e089805de3534cceb3c0d451",
        mapper: CartridgeMapper::Ascii8Sram2,
    },
    KnownCartridge {
        name: "Dragon Slayer 2 - Xanadu",
        digest: "709c6dbf170f0f77588c0f588042897499e9c4b8bba35fbcbf49c9658191eae4",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Shogun",
        digest: "20cc8e5c4f19ee2c6df65b907fdd5dd7209da45f49f019c727c3602ba4b03975",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Heroes of The Lance",
        digest: "3ece999cef6009359d303139836a213528d88fa04056a5151a470c55873af34c",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Japanese MSX-Write II",
        digest: "2e6d667ee76fd820d92e75587937a08ceeafbd43d76f2e70155195122ed32e19",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Siryousensen - War Of The Dead",
        digest: "3db3ba60c881bcd85793a5789259199743f05d88f8637b030009920e269fd5e7",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Taiyou no Shinden - Asteka II",
        digest: "ceee03f4262c209e56544e4bb12cd1271e17a75977ed856d473c325426d4603b",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Shougi Sinan 2",
        digest: "6725ee8e2c11b866ebab69e04e1e9c4035f84480eb96e54224a7616d8528107d",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Ultima III - Exodus",
        digest: "ec42c1316d11aaf2a78646693afd6552ce0e84cae01029fc58fe7d18a5b16a33",
        mapper: CartridgeMapper::Ascii8Sram8,
    },
    KnownCartridge {
        name: "Harry Fox - MSX Special",
        digest: "55dd2b2e0f1f164027d3aa9121fa29fd16fcda25680517909cc99806f13d886c",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "Hydlide 2 - Shine Of Darkness",
        digest: "29d336f6c0993bc9b2f34339ab140f49b965676c120cf8b32f50825853cc216b",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "Hydlide 2 - Shine Of Darkness",
        digest: "26863a1ee7f26d78fefd7c87f247062e895c38ccbbd0efcc68307e03a2cd1c19",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "Daisenryaku - Great Strategy",
        digest: "0000d3613a121128ab0a5543744ab883e1ecd937e90b137e5457c0df50a73036",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "Professional Mahjong Gokuh",
        digest: "8d40fef27588e88069fe4f80717030b1d2f1bdfa338e792cc758510b9a2fe695",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "Super Daisenryaku",
        digest: "55ee30da19769a102ab2fdd23fb7699db7f1ec52be2cfd35cfc4921ef387755a",
        mapper: CartridgeMapper::Ascii16Sram2,
    },
    KnownCartridge {
        name: "A-Train",
        digest: "0e9a48a9a44bfc173caf691e734610fbbce8ae4c2cc3d0156771ec1a424d6768",
        mapper: CartridgeMapper::Ascii16Sram8,
    },
    KnownCartridge {
        name: "Japanese MSX-Write",
        digest: "82f7adc6d72377bf55e208e501534671f828611e3cc6ab2419f638809ea9dec7",
        mapper: CartridgeMapper::MsxWrite,
    },
    KnownCartridge {
        name: "Genghis Khan",
        digest: "8a4d2269e74eca73603928d7a93cd38b8b589110feab5656a14a1a44a3f708c9",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Nobunaga no Yabou - Zenkokuhan",
        digest: "7971904f46a888b0bab20bc112a9d1f7b5c32723d539d623168b4f5139c6f939",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Genchohisi",
        digest: "87ac4b443aef3788a982ec41caabcd78c691bf700e1070c5c7d4709da89ea26d",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Genghis Khan",
        digest: "b8eb68716a60c8bac305f26be41d1b41fa1be02058e436251fa833f2ca30cef0",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Daikoukai Jidai",
        digest: "5f4c3d66602cffbfb8829327091d0a7bb4fe622704afc5951bf9f4dbb1c6c437",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "L'Empereur",
        digest: "a1dba3937d01c43c9d9237b800ab792a6f88dbe0dda122cb0b6469f7aac1bce8",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Europe War",
        digest: "0c5c32b7faf33820d7551f9d5973c39ad0dd798134386eb2282743d733e2fb24",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Inindo - Way of the Ninja",
        digest: "7662c166500456c211d581aef553d4388fcd91d79a5acffba07bdcc17e32c269",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Isin no Arashi",
        digest: "d814c407a0a0832867b9576d4ce260cd14a5c0880e1ccde8a4f37a9e21ab4d8a",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Nobunaga no Yabou - Bushouhuunroku",
        digest: "ef7ee8eec984fd7e62490f4baa7e53f25c7c08551470acb21e7efe9abcf97b0f",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Nobunaga no Yabou - Senkokugunyuden",
        digest: "8612e1485c572dce0d276f3bd3ab2297f43398408dce90db3abaeb87f6140485",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Nobunaga no Yabou - Zenkokuhan",
        digest: "88a53fb29d421f0cdee3e0ea0cf526ae0a63075be0d662a9c4b8fd23d1974da0",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Royal Blood",
        digest: "ebb1a6c696a7a2368d11b7ef52efe88a7a58117be93bd3ef512534305821a611",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Sangokushi 1 - Romance Of Three Kingdoms",
        digest: "fb13850d0ad465f9ce1c459355efe1db9588f0d901b4a82ffc9933b59ed34eec",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Sangokushi 2",
        digest: "ed774b43bfddcc079a6e2a4735661e17973b6ca4ca93287b79d6a5a65811617f",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Suikoden",
        digest: "41512f1416349530a3c84b1181d4455f02bd3fb22051ebd4beb04075d5f46104",
        mapper: CartridgeMapper::KoeiSram32,
    },
    KnownCartridge {
        name: "Teitoku no Ketsudan",
        digest: "eee7a8aa21f9cc270b3544bd2712b5856325e9c56303e7a2056f7c1b2bcf1394",
        mapper: CartridgeMapper::KoeiSram32,
    },
];

pub(super) fn mapper_for_digest(digest: &str) -> Option<CartridgeMapper> {
    KNOWN_CARTRIDGES
        .iter()
        .find(|known| known.digest == digest)
        .map(|known| {
            debug_assert!(!known.name.is_empty());
            known.mapper
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_hashes_resolve_without_a_heuristic() {
        assert!(KNOWN_CARTRIDGES.iter().all(|known| !known.name.is_empty()));
        assert_eq!(
            mapper_for_digest("563f2ce72d74936ef9190aa6cadda4741bed020755ff7e746c43e795e6928c6c"),
            Some(CartridgeMapper::Halnote)
        );
        assert_eq!(
            mapper_for_digest("083f965452828757c7859383a15ed224746190875d522229d41f5bd3cda3be98"),
            Some(CartridgeMapper::MsxDos2)
        );
        assert_eq!(mapper_for_digest("unknown"), None);
    }
}
