// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub(crate) const TRAMPOLINE_SIZE: usize = STOLEN + 14;

pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

pub(crate) static HITS: AtomicU64 = AtomicU64::new(0);

/// `jmp [rip+0]; dq hook; nop` - 15 bytes written over the function entry.
pub(crate) fn build_detour(hook: u64) -> [u8; STOLEN] {
    let mut b = [0x90u8; STOLEN];
    b[0] = 0xFF;
    b[1] = 0x25;
    b[2..6].copy_from_slice(&0u32.to_le_bytes());
    b[6..14].copy_from_slice(&hook.to_le_bytes());
    b
}

/// The stolen instructions followed by `jmp [rip+0]; dq resume`.
pub(crate) fn build_trampoline(stolen: &[u8], resume: u64) -> [u8; TRAMPOLINE_SIZE] {
    assert_eq!(stolen.len(), STOLEN);
    let mut b = [0u8; TRAMPOLINE_SIZE];
    b[..STOLEN].copy_from_slice(stolen);
    b[STOLEN] = 0xFF;
    b[STOLEN + 1] = 0x25;
    b[STOLEN + 2..STOLEN + 6].copy_from_slice(&0u32.to_le_bytes());
    b[STOLEN + 6..STOLEN + 14].copy_from_slice(&resume.to_le_bytes());
    b
}

/// Walks the executable sections of a loaded PE image (`base` = module
/// base) and returns the single match of [`SIG`].
pub(crate) unsafe fn scan_image(base: *const u8) -> Result<*mut u8, &'static str> {
    unsafe { scan_sig(base, SIG) }
}

/// Detours `target`. Returns the original bytes for [`uninstall`].
pub(crate) unsafe fn install(target: *mut u8, p: &Platform) -> Result<[u8; STOLEN], &'static str> {
    unsafe {
        let pat = parse_sig(SIG);
        let mut original = [0u8; STOLEN];
        original.copy_from_slice(std::slice::from_raw_parts(target, STOLEN));
        for (i, want) in pat.iter().take(STOLEN).enumerate() {
            if let Some(w) = want {
                if original[i] != *w {
                    return Err("function prologue does not match");
                }
            }
        }

        let tramp = (p.alloc_exec)(64);
        if tramp.is_null() {
            return Err("could not allocate executable memory");
        }
        let tb = build_trampoline(&original, target as u64 + STOLEN as u64);
        std::ptr::copy_nonoverlapping(tb.as_ptr(), tramp, tb.len());
        (p.flush)(tramp, tb.len());
        TRAMPOLINE.store(tramp as usize, Ordering::SeqCst);

        let Some(old) = (p.make_writable)(target, STOLEN) else {
            return Err("could not make the function writable");
        };
        let detour = build_detour(hook_set_water_level as *const () as usize as u64);
        std::ptr::copy_nonoverlapping(detour.as_ptr(), target, STOLEN);
        (p.restore_protect)(target, STOLEN, old);
        (p.flush)(target, STOLEN);
        Ok(original)
    }
}

pub(crate) unsafe fn uninstall(target: *mut u8, original: &[u8; STOLEN], p: &Platform) {
    unsafe {
        if let Some(old) = (p.make_writable)(target, STOLEN) {
            std::ptr::copy_nonoverlapping(original.as_ptr(), target, STOLEN);
            (p.restore_protect)(target, STOLEN, old);
            (p.flush)(target, STOLEN);
        }
    }
}

/// A detour of arbitrary stolen length.
pub(crate) struct Patch {
    pub target: usize,
    pub original: Vec<u8>,
    pub trampoline: usize,
}

/// Detours `target` (first `stolen` bytes must be whole, position
/// independent instructions and match `sig`). The trampoline address is
/// returned in the patch.
pub(crate) unsafe fn install_patch(
    target: *mut u8,
    sig: &str,
    stolen: usize,
    hook: usize,
    p: &Platform,
) -> Result<Patch, &'static str> {
    unsafe { install_patch_relocating(target, sig, stolen, &[], hook, p) }
}

/// `lea r64, [rip+disp32]` (7 bytes) at `at` in `code`, rewritten as `movabs r64, imm64`
/// (10 bytes) that loads the same address when the code runs from somewhere else.
pub(crate) fn relocate_rip_lea(code: &[u8], at: usize, origin: u64) -> Option<[u8; 10]> {
    let b = code.get(at..at + 7)?;
    if (b[0] & 0xFB) != 0x48 || b[1] != 0x8D || (b[2] & 0xC7) != 0x05 {
        return None;
    }
    let disp = i64::from(i32::from_le_bytes(b[3..7].try_into().ok()?));
    let value = (origin + at as u64 + 7).wrapping_add_signed(disp);
    let reg = ((b[0] >> 2) & 1) << 3 | ((b[2] >> 3) & 7);
    let mut out = [0u8; 10];
    out[0] = 0x48 | (reg >> 3);
    out[1] = 0xB8 | (reg & 7);
    out[2..].copy_from_slice(&value.to_le_bytes());
    Some(out)
}

/// Like [`install_patch`], for a prologue that holds `lea r64,[rip+disp32]` instructions
/// (at the byte offsets in `rip_leas`): those are rewritten in the trampoline.
pub(crate) unsafe fn install_patch_relocating(
    target: *mut u8,
    sig: &str,
    stolen: usize,
    rip_leas: &[usize],
    hook: usize,
    p: &Platform,
) -> Result<Patch, &'static str> {
    unsafe {
        assert!(stolen >= 14);
        let pat = parse_sig(sig);
        let original = std::slice::from_raw_parts(target, stolen).to_vec();
        for (i, want) in pat.iter().take(stolen).enumerate() {
            if let Some(w) = want {
                if original[i] != *w {
                    return Err("function prologue does not match");
                }
            }
        }
        let tramp = (p.alloc_exec)(64);
        if tramp.is_null() {
            return Err("could not allocate executable memory");
        }
        let mut tb = Vec::with_capacity(stolen + 32);
        let mut i = 0;
        while i < stolen {
            if rip_leas.contains(&i) {
                let Some(abs) = relocate_rip_lea(&original, i, target as u64) else {
                    return Err("prologue lea cannot be relocated");
                };
                tb.extend_from_slice(&abs);
                i += 7;
            } else {
                tb.push(original[i]);
                i += 1;
            }
        }
        tb.extend_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
        tb.extend_from_slice(&(target as u64 + stolen as u64).to_le_bytes());
        std::ptr::copy_nonoverlapping(tb.as_ptr(), tramp, tb.len());
        (p.flush)(tramp, tb.len());

        let mut detour = vec![0x90u8; stolen];
        detour[0] = 0xFF;
        detour[1] = 0x25;
        detour[2..6].copy_from_slice(&0u32.to_le_bytes()); // jmp [rip+0]
        detour[6..14].copy_from_slice(&(hook as u64).to_le_bytes());
        let Some(old) = (p.make_writable)(target, stolen) else {
            return Err("could not make the function writable");
        };
        std::ptr::copy_nonoverlapping(detour.as_ptr(), target, stolen);
        (p.restore_protect)(target, stolen, old);
        (p.flush)(target, stolen);
        Ok(Patch {
            target: target as usize,
            original,
            trampoline: tramp as usize,
        })
    }
}

pub(crate) unsafe fn uninstall_patch(patch: &Patch, p: &Platform) {
    unsafe {
        let target = patch.target as *mut u8;
        if let Some(old) = (p.make_writable)(target, patch.original.len()) {
            std::ptr::copy_nonoverlapping(patch.original.as_ptr(), target, patch.original.len());
            (p.restore_protect)(target, patch.original.len(), old);
            (p.flush)(target, patch.original.len());
        }
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) unsafe fn read_disp(at: *const u8, i: usize) -> usize {
    unsafe { at.add(i).cast::<u32>().read_unaligned() as usize }
}
