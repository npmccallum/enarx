// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use goblin::elf::{header::*, program_header::*, Elf};

use bounds::{Line, Span};
use memory::Page;

use std::cmp::{max, min};
use std::path::Path;

use super::Segment;

pub struct Component {
    pub segments: Vec<Segment>,
    pub entry: usize,
    pub pie: bool,
}

impl Component {
    /// Loads a binary from a file
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len() as usize;
        let (bytes, _unmap) = unsafe {
            let addr = mmap::map(0, len, libc::PROT_READ, libc::MAP_PRIVATE, Some(&file), 0)?;
            let bytes = std::slice::from_raw_parts(addr as *const u8, len);
            let unmap = mmap::Unmap::new(Span {
                start: addr,
                count: len,
            });
            (bytes, unmap)
        };

        Self::from_bytes(bytes)
    }

    /// Loads a binary from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        // Parse the file.
        let elf = Elf::parse(bytes).unwrap();

        // Validate identity assumptions.
        assert_eq!(elf.header.e_ident[EI_CLASS], ELFCLASS64);
        assert_eq!(elf.header.e_ident[EI_DATA], ELFDATA2LSB);
        assert_eq!(elf.header.e_ident[EI_VERSION], EV_CURRENT);

        // Validate header assumptions.
        assert_eq!(elf.header.e_machine, EM_X86_64);
        assert_eq!(elf.header.e_version, EV_CURRENT as _);
        let pie = match elf.header.e_type {
            ET_DYN => true,
            ET_EXEC => false,
            _ => panic!("Unsupported ELF type!"),
        };

        // Validate that there is no interpreter.
        assert!(!elf
            .program_headers
            .iter()
            .fold(false, |a, ph| a | (ph.p_type == PT_INTERP)));

        // Validate that the entry point is in one of the loaded sections.
        assert_eq!(
            1,
            elf.program_headers
                .iter()
                .filter(|ph| {
                    ph.p_type == PT_LOAD
                        && elf.header.e_entry >= ph.p_vaddr
                        && elf.header.e_entry < ph.p_vaddr + ph.p_memsz
                })
                .count()
        );

        let mut segments = Vec::new();
        for ph in elf.program_headers.iter() {
            if let Some(seg) = Segment::from_ph(&bytes, ph)? {
                segments.push(seg);
            }
        }

        // Validate that for pie binaries the first segment starts at 0.
        assert_eq!(pie, segments[0].dst == 0);

        Ok(Self {
            entry: elf.entry as _,
            segments,
            pie,
        })
    }

    /// Find the total memory region for the binary.
    pub fn region(&self) -> Line<usize> {
        self.segments
            .iter()
            .map(|x| Line {
                start: x.dst,
                end: x.dst + x.src.len() * Page::size(),
            })
            .fold(usize::max_value()..usize::min_value(), |l, r| {
                min(l.start, r.start)..max(l.end, r.end)
            })
            .into()
    }
}
