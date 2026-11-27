//! Load plan for a little-endian RV64I executable in an identity address space.

use std::{fmt, ops::Range};

#[derive(Debug, PartialEq, Eq)]
pub struct Image<'a> {
    pub entry: u64,
    pub stack: Range<u64>,
    pub segments: Vec<Segment<'a>>,
}

impl Image<'_> {
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
pub struct Segment<'a> {
    pub address: u64,
    pub data: &'a [u8],
    pub zero_fill: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ElfError {
    Header,
    ProgramHeaders,
    ProgramHeader { index: usize, reason: &'static str },
    Stack,
    StackOverlap(u64),
    SegmentOverlap(u64),
    Entry(u64),
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header => write!(f, "expected a little-endian ELF64 RISC-V LP64 executable"),
            Self::ProgramHeaders => write!(f, "invalid ELF64 program header table"),
            Self::ProgramHeader { index, reason } => {
                write!(f, "ELF program header {index}: {reason}")
            }
            Self::Stack => write!(f, "stack does not fit within the RAM interval"),
            Self::StackOverlap(address) => {
                write!(f, "load segment at {address:#x} overlaps the stack")
            }
            Self::SegmentOverlap(address) => write!(f, "load segments overlap at {address:#x}"),
            Self::Entry(address) => write!(
                f,
                "entry {address:#x} must name an aligned instruction in an executable segment"
            ),
        }
    }
}

impl std::error::Error for ElfError {}

pub fn load(bytes: &[u8], ram: Range<u64>, stack_size: u64) -> Result<Image<'_>, ElfError> {
    const HEADER_SIZE: usize = 64;
    const PROGRAM_HEADER_SIZE: usize = 56;
    const PT_LOAD: u32 = 1;
    const PF_X: u32 = 1;

    let header = Record(bytes.get(..HEADER_SIZE).ok_or(ElfError::Header)?);
    if &header.0[..7] != b"\x7fELF\x02\x01\x01"
        || header.u16(16) != 2 // ET_EXEC
        || header.u16(18) != 243 // EM_RISCV
        || header.u32(20) != 1 // EV_CURRENT
        || header.u32(48) != 0 // RV64I / LP64 ELF flags
        || usize::from(header.u16(52)) != HEADER_SIZE
    {
        return Err(ElfError::Header);
    }

    let entry = header.u64(24);
    let phoff = header.u64(32);
    let phnum = header.u16(56);
    if phoff < HEADER_SIZE as u64
        || usize::from(header.u16(54)) != PROGRAM_HEADER_SIZE
        || phnum == 0
        || phnum == u16::MAX
    {
        return Err(ElfError::ProgramHeaders);
    }
    let phdrs = file_region(bytes, phoff, u64::from(phnum) * PROGRAM_HEADER_SIZE as u64)
        .ok_or(ElfError::ProgramHeaders)?;

    let stack_top = ram.end & !15;
    let stack_bottom = stack_top
        .checked_sub(stack_size)
        .filter(|&bottom| stack_size != 0 && bottom >= ram.start)
        .ok_or(ElfError::Stack)?;
    let mut segments = Vec::new();
    let mut entry_mapped = false;
    for (index, phdr) in phdrs.chunks_exact(PROGRAM_HEADER_SIZE).enumerate() {
        let phdr = Record(phdr);
        let error = |reason| ElfError::ProgramHeader { index, reason };
        match phdr.u32(0) {
            PT_LOAD => {}
            2 | 3 => return Err(error("dynamic linking metadata in executable")), // PT_DYNAMIC, PT_INTERP
            _ => continue,
        }
        let offset = phdr.u64(8);
        let address = phdr.u64(16); // p_vaddr, the address used by the program
        let file_size = phdr.u64(32);
        let mem_size = phdr.u64(40);
        let align = phdr.u64(48);
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
            return Err(ElfError::StackOverlap(address));
        }
        let data = if file_size == 0 {
            &[]
        } else {
            file_region(bytes, offset, file_size)
                .ok_or_else(|| error("segment data outside file"))?
        };
        entry_mapped |= phdr.u32(4) & PF_X != 0
            && entry >= address
            && entry
                .checked_add(4)
                .is_some_and(|fetch_end| fetch_end <= end);
        segments.push((
            Segment {
                address,
                data,
                zero_fill: mem_size - file_size,
            },
            end,
        ));
    }

    segments.sort_unstable_by_key(|(segment, _)| segment.address);
    for pair in segments.windows(2) {
        if pair[0].1 > pair[1].0.address {
            return Err(ElfError::SegmentOverlap(pair[1].0.address));
        }
    }
    if entry & 3 != 0 || !entry_mapped {
        return Err(ElfError::Entry(entry));
    }
    Ok(Image {
        entry,
        stack: stack_bottom..stack_top,
        segments: segments.into_iter().map(|(segment, _)| segment).collect(),
    })
}

fn file_region(bytes: &[u8], start: u64, size: u64) -> Option<&[u8]> {
    let end = usize::try_from(start.checked_add(size)?).ok()?;
    bytes.get(usize::try_from(start).ok()?..end)
}

/// A complete ELF header or program header, with fixed-size fields at known offsets.
struct Record<'a>(&'a [u8]);

impl Record<'_> {
    fn u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes(self.0[offset..offset + 2].try_into().unwrap())
    }

    fn u32(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    fn u64(&self, offset: usize) -> u64 {
        u64::from_le_bytes(self.0[offset..offset + 8].try_into().unwrap())
    }
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
                    data: &[0x93, 0x01, 0x70, 0],
                    zero_fill: 0
                },
                Segment {
                    address: 0x1804,
                    data: &[0xfe, 0xca],
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
                data: &[],
                zero_fill: 10
            }
        );
    }

    #[test]
    fn malformed_headers_return_errors_for_short_or_wrapping_ranges() {
        let mut bytes = executable();
        assert_eq!(
            load(&bytes[..63], 0x1000..0x3000, 0x100),
            Err(ElfError::Header)
        );
        assert_eq!(
            load(&bytes[..175], 0x1000..0x3000, 0x100),
            Err(ElfError::ProgramHeaders)
        );
        put64(&mut bytes, 32, u64::MAX - 7);
        assert_eq!(
            load(&bytes, 0x1000..0x3000, 0x100),
            Err(ElfError::ProgramHeaders)
        );
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
            Err(ElfError::StackOverlap(0x1804))
        );
        for entry in [0x1002, 0x1004, 0x1804] {
            let mut invalid = bytes.clone();
            put64(&mut invalid, 24, entry);
            assert_eq!(
                load(&invalid, ram.clone(), 0x100),
                Err(ElfError::Entry(entry))
            );
        }
        let mut overlap = bytes;
        put64(&mut overlap, 80, 0x1000);
        assert_eq!(
            load(&overlap, ram, 0x100),
            Err(ElfError::SegmentOverlap(0x1000))
        );
    }
}
