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
    pm32_0028_c00325a6_dd,
    real_0213_000046c0_f3_67_66_a5,
    smoke_real_add_byte_memory,
    vm86_094e_0000123c_66_9c,
    vm86_2ecc_00000053_dc,
    vm86_2ecc_00000057_dc,
    x87_fadd_denormal_unmasked,
    x87_fadd_m32_fault_keeps_pointers,
    x87_fcomp_snan_unmasked,
    x87_fdivp_zero_divide_unmasked,
    x87_fiadd_m16_empty_reads_operand,
    x87_fistp_m16_out_of_range_unmasked,
    x87_fld1_stack_overflow_unmasked,
    x87_fld_m32_denormal_unmasked,
    x87_fld_m32_snan_unmasked,
    x87_fmul_overflow_unmasked,
    x87_fmul_underflow_unmasked,
    x87_fpatan_denormal_unmasked,
    x87_fprem_invalid_unmasked_clears_c2,
    x87_fscale_denormal_round_up,
    x87_fscale_overflow_round_to_zero,
    x87_fscale_overflow_unmasked,
    x87_fsincos_stack_overflow_unmasked,
    x87_fsqrt_negative_denormal_priority,
    x87_fsqrt_negative_unmasked,
    x87_fst_m32_snan_unmasked,
    x87_fst_m32_snan_unmasked_no_access,
    x87_fstp_m32_empty_unmasked,
    x87_fstp_m32_overflow_unmasked,
    x87_fstp_st1_empty_unmasked,
}
