//! One-step cases of the 486 core with expectations from the Bochs oracle. Each case file
//! in `oracle_i486/cases/` holds the initial state, the memory and port values the step
//! sees and the expected result. The format is described in `oracle_i486/case.rs`.

#[path = "oracle_i486/case.rs"]
pub mod case;

/// Generates one test function per case file `oracle_i486/cases/<name>.case`.
macro_rules! oracle_cases {
    ($($name:ident),* $(,)?) => {
        $(
            #[doc = concat!("Runs the oracle case `", stringify!($name), ".case`.")]
            #[test]
            fn $name() {
                case::check(
                    concat!(stringify!($name), ".case"),
                    include_str!(concat!("oracle_i486/cases/", stringify!($name), ".case")),
                );
            }
        )*
    };
}

oracle_cases! {
    pm16_0117_00000175_83,
    pm16_0117_000026a6_66_61,
    real_0213_000046c0_f3_67_66_a5,
    smoke_real_add_byte_memory,
    vm86_094e_0000123c_66_9c,
}
