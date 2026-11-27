//! Load plan for a little-endian RV64I executable in an identity address space.

use object::{
    LittleEndian as Le, elf,
    read::elf::{FileHeader, ProgramHeader},
};
use snafu::{ResultExt, Snafu};
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    pub entry: u64,
    pub stack: Range<u64>,
    pub segments: Vec<Segment>,
}

impl Image {
    pub fn initial_sp(&self) -> u64 {
        self.stack.end
    }

    pub fn initial_registers(&self) -> [u64; 32] {
        let mut registers = [0; 32];
        registers[2] = self.initial_sp();
        registers
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    pub address: u64,
    pub data: Vec<u8>,
    pub zero_fill: u64,
}

#[derive(Debug, PartialEq, Eq, Snafu)]
pub enum ElfError {
    #[snafu(display("{region}: {source}"))]
    Reader {
        region: &'static str,
        source: object::Error,
    },
    #[snafu(display("expected a little-endian ELF64 RISC-V LP64 executable"))]
    Header,
    #[snafu(display("invalid ELF64 program header table"))]
    ProgramHeaders,
    #[snafu(display("ELF program header {index}: {reason}"))]
    ProgramHeader { index: usize, reason: &'static str },
    #[snafu(display("stack does not fit within the RAM interval"))]
    Stack,
    #[snafu(display("load segment at {address:#x} overlaps the stack"))]
    StackOverlap { address: u64 },
    #[snafu(display("load segments overlap at {address:#x}"))]
    SegmentOverlap { address: u64 },
    #[snafu(display(
        "entry {address:#x} must name an aligned instruction in an executable segment"
    ))]
    Entry { address: u64 },
}

pub fn load(bytes: &[u8], ram: Range<u64>, stack_size: u64) -> Result<Image, ElfError> {
    let sp = ram.end & !15;
    load_at(bytes, ram, sp, stack_size)
}

pub(crate) fn load_at(
    bytes: &[u8],
    ram: Range<u64>,
    stack_top: u64,
    stack_size: u64,
) -> Result<Image, ElfError> {
    type Header = elf::FileHeader64<Le>;
    let header = Header::parse(bytes).context(ReaderSnafu {
        region: "ELF64 header",
    })?;
    if !header.is_little_endian()
        || header.e_type(Le) != elf::ET_EXEC
        || header.e_machine(Le) != elf::EM_RISCV
        || header.e_version(Le) != u32::from(elf::EV_CURRENT)
        || header.e_flags(Le) != 0
        || usize::from(header.e_ehsize(Le)) != size_of::<Header>()
    {
        return Err(ElfError::Header);
    }
    let entry = header.e_entry(Le);
    if header.e_phoff(Le) < size_of::<Header>() as u64 || matches!(header.e_phnum(Le), 0 | u16::MAX)
    {
        return Err(ElfError::ProgramHeaders);
    }
    let phdrs = header.program_headers(Le, bytes).context(ReaderSnafu {
        region: "ELF64 program header table",
    })?;
    let stack_bottom = stack_top
        .checked_sub(stack_size)
        .filter(|&bottom| stack_size != 0 && bottom >= ram.start && stack_top <= ram.end)
        .ok_or(ElfError::Stack)?;
    let mut segments = Vec::new();
    let mut entry_mapped = false;
    for (index, phdr) in phdrs.iter().enumerate() {
        let error = |reason| ElfError::ProgramHeader { index, reason };
        match phdr.p_type(Le) {
            elf::PT_LOAD => {}
            elf::PT_DYNAMIC | elf::PT_INTERP => {
                return Err(error("dynamic linking metadata in executable"));
            } // PT_DYNAMIC, PT_INTERP
            _ => continue,
        }
        let offset = phdr.p_offset(Le);
        let address = phdr.p_vaddr(Le); // p_vaddr, the address used by the program
        let file_size = phdr.p_filesz(Le);
        let mem_size = phdr.p_memsz(Le);
        let align = phdr.p_align(Le);
        if file_size > mem_size {
            return Err(error("p_filesz exceeds p_memsz"));
        }
        if align > 1 && (!align.is_power_of_two() || address % align != offset % align) {
            return Err(error("invalid p_align or incongruent p_vaddr and p_offset"));
        }
        if mem_size == 0 {
            continue;
        }
        let end = address
            .checked_add(mem_size)
            .ok_or_else(|| error("load address overflow"))?;
        if address < ram.start || end > ram.end {
            return Err(error("load segment outside RAM"));
        }
        if address < stack_top && end > stack_bottom {
            return Err(ElfError::StackOverlap { address });
        }
        let data = if file_size == 0 {
            &[]
        } else {
            phdr.data(Le, bytes)
                .map_err(|()| error("segment data outside file"))?
        };
        entry_mapped |= phdr.p_flags(Le) & elf::PF_X != 0
            && entry >= address
            && entry
                .checked_add(4)
                .is_some_and(|fetch_end| fetch_end <= end);
        segments.push((address..end, data, mem_size - file_size));
    }

    segments.sort_unstable_by_key(|(range, _, _)| range.start);
    for pair in segments.windows(2) {
        if pair[0].0.end > pair[1].0.start {
            return Err(ElfError::SegmentOverlap {
                address: pair[1].0.start,
            });
        }
    }
    if entry & 3 != 0 || !entry_mapped {
        return Err(ElfError::Entry { address: entry });
    }
    Ok(Image {
        entry,
        stack: stack_bottom..stack_top,
        segments: segments
            .into_iter()
            .map(|(range, data, zero_fill)| Segment {
                address: range.start,
                data: data.to_vec(),
                zero_fill,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn executable() -> Vec<u8> {
        let mut bytes = vec![0; 0x204];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        put16(&mut bytes, 16, 2);
        put16(&mut bytes, 18, 243);
        put32(&mut bytes, 20, 1);
        put64(&mut bytes, 24, 0x1000);
        put64(&mut bytes, 32, 64);
        put16(&mut bytes, 52, 64);
        put16(&mut bytes, 54, 56);
        put16(&mut bytes, 56, 2);

        // Data first in the table; contains initialized data and a BSS tail.
        put32(&mut bytes, 64, 1);
        put32(&mut bytes, 68, 6);
        put64(&mut bytes, 72, 0x204);
        put64(&mut bytes, 80, 0x1804);
        put64(&mut bytes, 88, 0x7804);
        put64(&mut bytes, 96, 2);
        put64(&mut bytes, 104, 10);
        put64(&mut bytes, 112, 4);

        put32(&mut bytes, 120, 1);
        put32(&mut bytes, 124, 5);
        put64(&mut bytes, 128, 0x200);
        put64(&mut bytes, 136, 0x1000);
        put64(&mut bytes, 144, 0x7000);
        put64(&mut bytes, 152, 4);
        put64(&mut bytes, 160, 4);
        put64(&mut bytes, 168, 0x100);
        bytes[0x200..].copy_from_slice(&[0x93, 0x01, 0x70, 0]);
        bytes.extend_from_slice(&[0xfe, 0xca]);
        bytes
    }

    #[test]
    fn load_plan_names_entry_stack_data_and_bss_tails() {
        let bytes = executable();
        let image = load(&bytes, 0x1000..0x300d, 0x100).unwrap();
        assert_eq!(image.entry, 0x1000);
        assert_eq!(image.stack, 0x2f00..0x3000);
        let mut registers = [0; 32];
        registers[2] = 0x3000;
        assert_eq!(image.initial_registers(), registers);
        assert_eq!(
            image.segments,
            [
                Segment {
                    address: 0x1000,
                    data: vec![0x93, 0x01, 0x70, 0],
                    zero_fill: 0
                },
                Segment {
                    address: 0x1804,
                    data: vec![0xfe, 0xca],
                    zero_fill: 8
                },
            ]
        );
    }

    #[test]
    fn bss_only_load_segment_does_not_require_file_payload() {
        let mut bytes = executable();
        put64(&mut bytes, 72, 0x400);
        put64(&mut bytes, 96, 0);
        let image = load(&bytes, 0x1000..0x3000, 0x100).unwrap();
        assert_eq!(
            image.segments[1],
            Segment {
                address: 0x1804,
                data: vec![],
                zero_fill: 10
            }
        );
    }

    #[test]
    fn malformed_headers_return_errors_for_short_or_wrapping_ranges() {
        let mut bytes = executable();
        assert!(matches!(
            load(&bytes[..63], 0x1000..0x3000, 0x100),
            Err(ElfError::Reader { .. })
        ));
        assert!(matches!(
            load(&bytes[..175], 0x1000..0x3000, 0x100),
            Err(ElfError::Reader { .. })
        ));
        put64(&mut bytes, 32, u64::MAX - 7);
        assert!(matches!(
            load(&bytes, 0x1000..0x3000, 0x100),
            Err(ElfError::Reader { .. })
        ));
        bytes[5] = 2;
        assert_eq!(load(&bytes, 0x1000..0x3000, 0x100), Err(ElfError::Header));
    }

    #[test]
    fn load_rejects_invalid_sizes_and_segment_address_overflow() {
        let ram = 0x1000..0x3000;
        let mut bytes = executable();
        for (field, value, reason) in [
            (96, 11, "p_filesz exceeds p_memsz"),
            (96, 3, "segment data outside file"),
            (72, u64::MAX - 3, "segment data outside file"),
            (80, u64::MAX - 3, "load address overflow"),
            (80, 0x804, "load segment outside RAM"),
            (
                80,
                0x1805,
                "invalid p_align or incongruent p_vaddr and p_offset",
            ),
        ] {
            let mut invalid = bytes.clone();
            put64(&mut invalid, field, value);
            assert_eq!(
                load(&invalid, ram.clone(), 0x100),
                Err(ElfError::ProgramHeader { index: 0, reason })
            );
        }
        put64(&mut bytes, 72, u64::MAX - 3);
        put64(&mut bytes, 96, 4);
        assert_eq!(
            load(&bytes, ram, 0x100),
            Err(ElfError::ProgramHeader {
                index: 0,
                reason: "segment data outside file"
            })
        );
    }

    #[test]
    fn load_checks_stack_segment_layout_and_entry_mapping() {
        let ram = 0x1000..0x3000;
        let bytes = executable();
        assert_eq!(load(&bytes, ram.clone(), 0), Err(ElfError::Stack));
        assert_eq!(load(&bytes, ram.clone(), 0x2001), Err(ElfError::Stack));
        assert_eq!(
            load(&bytes, ram.clone(), 0x1800),
            Err(ElfError::StackOverlap { address: 0x1804 })
        );
        for entry in [0x1002, 0x1004, 0x1804] {
            let mut invalid = bytes.clone();
            put64(&mut invalid, 24, entry);
            assert_eq!(
                load(&invalid, ram.clone(), 0x100),
                Err(ElfError::Entry { address: entry })
            );
        }
        let mut overlap = bytes;
        put64(&mut overlap, 80, 0x1000);
        assert_eq!(
            load(&overlap, ram, 0x100),
            Err(ElfError::SegmentOverlap { address: 0x1000 })
        );
    }
}
