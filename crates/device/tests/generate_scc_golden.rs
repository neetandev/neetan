//! Regenerates SCC vectors from the Rust implementation.

use std::{fmt::Write as _, path::PathBuf};

use device::scc::{SccPlus, StandardScc};

/// Generated vector file relative to the device crate.
const GOLDEN_OUTPUT: &str = "tests/golden/scc.rs";

fn collect<const VARIANT: u8>(scc: &mut device::scc::Scc<VARIANT>, count: usize) -> Vec<i16> {
    (0..count).map(|_| scc.clock()).collect()
}

fn write_vector(output: &mut String, name: &str, description: &str, samples: &[i16]) {
    writeln!(output, "/// {description}").unwrap();
    writeln!(output, "pub const {name}: &[i16] = &[").unwrap();
    for chunk in samples.chunks(16) {
        output.push_str("    ");
        for sample in chunk {
            write!(output, "{sample}, ").unwrap();
        }
        output.push('\n');
    }
    output.push_str("];\n\n");
}

fn write_standard_frequency(scc: &mut StandardScc, channel: usize, frequency: u16) {
    scc.write(0x80 + (channel * 2) as u8, frequency as u8);
    scc.write(0x81 + (channel * 2) as u8, (frequency >> 8) as u8);
}

#[test]
#[ignore = "regenerates committed SCC golden vectors"]
fn generate_scc_golden_vectors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut output = String::from(
        "// Initial vectors were established from the BSD-licensed MAME K051649/K052539 core.\n\
         // Regenerate: cargo test -p device --test generate_scc_golden -- --ignored\n\n",
    );

    let mut silence = StandardScc::new();
    write_vector(
        &mut output,
        "RESET_SILENCE",
        "Raw SCC clocks after reset.",
        &collect(&mut silence, 32),
    );

    let mut signed = StandardScc::new();
    for index in 0..32 {
        signed.write(index, (index as i16 * 8 - 128) as u8);
    }
    write_standard_frequency(&mut signed, 0, 9);
    signed.write(0x8A, 15);
    signed.write(0x8F, 1);
    write_vector(
        &mut output,
        "SIGNED_WAVEFORM",
        "Raw SCC clocks for a signed waveform.",
        &collect(&mut signed, 64),
    );

    let mut mixed = StandardScc::new();
    for channel in 0..5 {
        for index in 0..32 {
            let address = if channel == 4 {
                0x60 + index
            } else {
                channel * 0x20 + index
            };
            mixed.write(address as u8, (16 * channel + index) as u8);
        }
        write_standard_frequency(&mut mixed, channel, 9 + channel as u16);
        mixed.write(0x8A + channel as u8, 15 - channel as u8);
    }
    mixed.write(0x8F, 0x1F);
    write_vector(
        &mut output,
        "FIVE_CHANNEL_MIX",
        "Raw SCC clocks for all five channels.",
        &collect(&mut mixed, 64),
    );

    let mut halted = StandardScc::new();
    halted.write(0, 127);
    write_standard_frequency(&mut halted, 0, 8);
    halted.write(0x8F, 1);
    write_vector(
        &mut output,
        "HALTED_PERIOD",
        "Raw SCC clocks for the halted period boundary.",
        &collect(&mut halted, 16),
    );

    let mut plus = SccPlus::new();
    plus.write(0x60, 16);
    plus.write(0x80, 64);
    for channel in [3, 4] {
        plus.write(0xA0 + (channel * 2) as u8, 9);
        plus.write(0xA1 + (channel * 2) as u8, 0);
        plus.write(0xAA + channel as u8, 15);
    }
    plus.write(0xAF, 0x18);
    write_vector(
        &mut output,
        "PLUS_INDEPENDENT_WAVEFORMS",
        "Raw SCC+ clocks for independent channel four and five waveforms.",
        &collect(&mut plus, 32),
    );

    std::fs::write(root.join(GOLDEN_OUTPUT), output).expect("write SCC vectors");
}
