// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

//! What a signature match tells us about the code around it, read by decoding its
//! instructions with `iced-x86` instead of counting to a byte offset by hand. A game update
//! that shifts bytes inside a function then moves the offsets with it, and a match that is
//! not the code we expect fails with a reason instead of yielding a garbage address.
//!
//! The functions that interpret instructions take a slice of decoded instructions, so they
//! can be tested on hand-written bytes as well as on the real binaries.

#[cfg(any(windows, test, all(target_os = "linux", feature = "server")))]
use iced_x86::Register;
use iced_x86::{Code, Decoder, DecoderOptions, Instruction};

/// Decodes `bytes` as x86-64 starting at address `ip`, so RIP-relative operands resolve to
/// real addresses. Stops at the first undecodable instruction (a signature may end in the
/// middle of one).
pub(crate) fn decode_bytes(bytes: &[u8], ip: u64) -> Vec<Instruction> {
    let mut decoder = Decoder::with_ip(64, bytes, ip, DecoderOptions::NONE);
    let mut out = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            break;
        }
        out.push(instruction);
    }
    out
}

/// Decodes the bytes `sig` matched at `at`. Every instruction in the match begins at a
/// boundary, because all signatures start at one.
///
/// # Safety
/// `at` must be the start of a match of `sig`, so that its bytes are readable.
pub(crate) unsafe fn decode_match(at: *const u8, sig: &str) -> Vec<Instruction> {
    let len = sig.split_whitespace().count();
    // SAFETY: the match covers `len` bytes of the image.
    let bytes = unsafe { std::slice::from_raw_parts(at, len) };
    decode_bytes(bytes, at as u64)
}

/// Addresses loaded with `mov r64,[rip+x]` or `lea r64,[rip+x]`: the engine's globals.
fn global_loads(code: &[Instruction]) -> impl Iterator<Item = usize> + '_ {
    code.iter()
        .filter(|i| {
            matches!(i.code(), Code::Mov_r64_rm64 | Code::Lea_r64_m) && i.is_ip_rel_memory_operand()
        })
        .map(|i| i.ip_rel_memory_address() as usize)
}

/// The global that the match's only RIP-relative load refers to.
#[cfg(feature = "server")]
pub(crate) fn only_global(code: &[Instruction]) -> Result<usize, &'static str> {
    let mut loads = global_loads(code);
    match (loads.next(), loads.next()) {
        (Some(global), None) => Ok(global),
        (None, _) => Err("the matched code does not load a global"),
        _ => Err("the matched code loads more than one global"),
    }
}

/// The global that the match's last RIP-relative load refers to.
pub(crate) fn last_global(code: &[Instruction]) -> Result<usize, &'static str> {
    global_loads(code)
        .last()
        .ok_or("the matched code does not load a global")
}

/// The displacement of a memory operand addressed off a general register (not RIP or RSP),
/// if it is an offset into a game object rather than a stack slot or a small field.
#[cfg(all(feature = "server", any(windows, test, target_os = "linux")))]
fn object_field(i: &Instruction) -> Option<usize> {
    let base = i.memory_base();
    if matches!(base, Register::None | Register::RIP | Register::RSP) {
        return None;
    }
    let disp = i.memory_displacement64() as i64;
    usize::try_from(disp)
        .ok()
        .filter(|d| (0x100..0x1000).contains(d))
}

#[cfg(all(feature = "server", any(windows, test, target_os = "linux")))]
const LAYOUT_ERROR: &str = "the entity layout is not the expected one (different game build?)";

/// `m_nWaterLevel`'s offset in `CBaseEntity::SetWaterLevel` on Windows: the `movzx`, `cmp`,
/// `lea` and `mov` that name it must all agree (the vtable `call` in between is not a field).
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) fn water_level_offset(code: &[Instruction]) -> Result<usize, &'static str> {
    water_level_offset_in(code, 4)
}

/// The same for `places` field accesses. GCC compares the level in registers
/// (`cmp r13b,sil`), so the Linux function names the field in three places, not four.
#[cfg(all(feature = "server", any(windows, test, target_os = "linux")))]
pub(crate) fn water_level_offset_in(
    code: &[Instruction],
    places: usize,
) -> Result<usize, &'static str> {
    let mut offsets = code
        .iter()
        .filter(|i| {
            matches!(
                i.code(),
                Code::Movzx_r32_rm8 | Code::Cmp_rm8_r8 | Code::Lea_r64_m | Code::Mov_rm8_r8
            )
        })
        .filter_map(object_field);
    let first = offsets.next().ok_or(LAYOUT_ERROR)?;
    let mut count = 1;
    for offset in offsets {
        if offset != first {
            return Err(LAYOUT_ERROR);
        }
        count += 1;
    }
    if count == places {
        Ok(first)
    } else {
        Err(LAYOUT_ERROR)
    }
}

/// `m_iEFlags`' offset in `CCollisionEvent::FluidEndTouch`: the `and [entity+x],
/// ~EFL_TOUCHING_FLUID`.
#[cfg(all(feature = "server", any(windows, test, target_os = "linux")))]
fn fluid_flags(code: &[Instruction]) -> Option<(usize, usize)> {
    const NOT_TOUCHING_FLUID: u32 = 0xFFF7_FFFF;
    let at = code
        .iter()
        .position(|i| i.code() == Code::And_rm32_imm32 && i.immediate32() == NOT_TOUCHING_FLUID)?;
    object_field(&code[at]).map(|flags| (at, flags))
}

/// A vtable displacement, as a slot (the displacement is a multiple of 8, and is the
/// engine's own entity vtable, which is large).
#[cfg(all(feature = "server", any(windows, test, target_os = "linux")))]
fn entity_slot(i: &Instruction) -> Option<usize> {
    usize::try_from(i.memory_displacement64() as i64)
        .ok()
        .filter(|d| d.is_multiple_of(8) && (0x100..0x1000).contains(d))
        .map(|d| d / 8)
}

/// From the Windows `CCollisionEvent::FluidEndTouch`: `m_iEFlags`' offset and the
/// `OnEntityEvent` vtable slot (the last indirect call).
#[cfg(all(feature = "server", any(windows, test)))]
pub(crate) fn fluid_layout(code: &[Instruction]) -> Result<(usize, usize), &'static str> {
    let (_, flags) = fluid_flags(code).ok_or(LAYOUT_ERROR)?;
    let event = code
        .iter()
        .rev()
        .find(|i| i.code() == Code::Call_rm64 && i.memory_base() != Register::None)
        .and_then(entity_slot)
        .ok_or(LAYOUT_ERROR)?;
    Ok((flags, event))
}

/// From the Linux `CCollisionEvent::FluidEndTouch`: `m_iEFlags`' offset and the
/// `OnEntityEvent` vtable slot. GCC loads the function pointer into a register
/// (`mov r12,[rax+x]`, then `call r12` after the arguments are set up), so the slot is the
/// first such load after the flag is cleared.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
pub(crate) fn fluid_layout_via_load(code: &[Instruction]) -> Result<(usize, usize), &'static str> {
    let (at, flags) = fluid_flags(code).ok_or(LAYOUT_ERROR)?;
    let event = code[at..]
        .iter()
        .find(|i| i.code() == Code::Mov_r64_rm64 && i.memory_base() != Register::None)
        .and_then(entity_slot)
        .ok_or(LAYOUT_ERROR)?;
    Ok((flags, event))
}

/// Where the `Ray_t` is, relative to the stack pointer, in the engine's helper that builds
/// one from two points. Read from the disassembly; not part of the matched bytes.
#[cfg(any(windows, test))]
const RAY_OFFSET_IN_FRAME: i64 = 0x40;

/// `Ray_t::m_IsSwept`'s offset, from the ray helper (`mov rax,rsp; push rbx; sub rsp,N`,
/// then `mov byte [rax+disp],1` with the `Ray_t` at `rsp+0x40`): the byte lies `disp` from
/// the entry stack pointer, and the `Ray_t` starts `8 * pushes + N - 0x40` below it.
#[cfg(any(windows, test))]
pub(crate) fn ray_swept_offset(code: &[Instruction]) -> Result<usize, &'static str> {
    const WRONG: &str = "the ray helper is not the expected code (different game build?)";
    let mut pushes = 0i64;
    let mut frame = None;
    for i in code {
        match i.code() {
            Code::Push_r64 if frame.is_none() => pushes += 1,
            Code::Sub_rm64_imm8 | Code::Sub_rm64_imm32
                if frame.is_none() && i.op0_register() == Register::RSP =>
            {
                frame = Some(i.immediate(1) as i64);
            }
            _ => {}
        }
    }
    let frame = frame.ok_or(WRONG)?;
    let disp = code
        .iter()
        .find(|i| {
            i.code() == Code::Mov_rm8_imm8
                && i.memory_base() == Register::RAX
                && i.immediate8() == 1
        })
        .map(|i| i.memory_displacement64() as i64)
        .ok_or(WRONG)?;
    usize::try_from(disp + 8 * pushes + frame - RAY_OFFSET_IN_FRAME)
        .ok()
        .filter(|o| (0x30..0x80).contains(o))
        .ok_or(WRONG)
}
