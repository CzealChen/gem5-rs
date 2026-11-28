//! Static RV64I object audit using the blocking core's actual semantics.
use gem5rs::{m5, riscv};
use object::{Object, ObjectSection, ObjectSymbol, SectionKind};
use serde_json::json;

fn classification(word: u32) -> &'static str {
    if m5::Operation::decode(word).is_some() {
        return "m5_requires_enabled_platform";
    }
    match riscv::execute(0, word, &[0; 32]) {
        Err(riscv::Fault {
            reason: riscv::FaultReason::IllegalInstruction(_),
            ..
        }) => "unsupported",
        Err(riscv::Fault {
            reason: riscv::FaultReason::EnvironmentCall | riscv::FaultReason::Breakpoint,
            ..
        }) => "terminal_trap",
        // Alignment depends on operands, not instruction support. No memory
        // transaction is issued by execute(), and no architectural state changes.
        _ => "supported",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut reports = Vec::new();
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path)?;
        let file = object::File::parse(&*bytes)?;
        if file.architecture() != object::Architecture::Riscv64 || !file.is_little_endian() {
            return Err(format!("expected little-endian RV64 object: {path}").into());
        }
        let mut findings = Vec::new();
        let mut count = 0;
        for section in file.sections().filter(|s| s.kind() == SectionKind::Text) {
            let data = section.data()?;
            let mut offset = 0;
            while offset < data.len() {
                // The platform uses uncompressed RV64I. Report short/other
                // encodings explicitly rather than interpreting them as RV64I.
                let length = if data.len() - offset >= 2 && data[offset] & 3 != 3 {
                    2
                } else {
                    4
                };
                if data.len() - offset < length {
                    return Err(format!("truncated instruction in {path}").into());
                }
                let mut raw = [0; 4];
                raw[..length].copy_from_slice(&data[offset..offset + length]);
                let word = u32::from_le_bytes(raw);
                let status = if length == 2 {
                    "unsupported"
                } else {
                    classification(word)
                };
                if status != "supported" {
                    findings.push(json!({
                        "section": section.name()?, "offset": offset,
                        "word": format!("0x{word:08x}"), "status": status,
                    }));
                }
                count += 1;
                offset += length;
            }
        }
        let mut undefined = Vec::new();
        let mut defined = Vec::new();
        for symbol in file.symbols() {
            if !symbol.is_global() || symbol.name()?.is_empty() {
                continue;
            }
            if symbol.is_undefined() {
                undefined.push(symbol.name()?.to_owned());
            } else {
                defined.push(symbol.name()?.to_owned());
            }
        }
        reports.push(
            json!({"path": path, "instructions": count, "findings": findings,
                            "undefined": undefined, "defined": defined}),
        );
    }
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::classification;

    #[test]
    fn distinguishes_isa_gaps_from_operands_and_platform_traps() {
        assert_eq!(classification(0x0010_0093), "supported"); // addi
        assert_eq!(classification(0x0010_3083), "supported"); // ld from misaligned address
        for word in [
            0x0220_81b3,
            0x0010_1173,
            0x3020_0073,
            0x0000_100f,
            0x0000_0001,
        ] {
            assert_eq!(classification(word), "unsupported"); // mul, CSR, mret, fence.i, C
        }
        assert_eq!(classification(0x0000_0073), "terminal_trap");
        assert_eq!(classification(0x0010_0073), "terminal_trap");
        assert_eq!(classification(0x4200_007b), "m5_requires_enabled_platform");
    }
}
