// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::sync::atomic::Ordering;

pub(crate) const OFF_PEV: usize = 0x30;

pub(crate) const OFF_EDICT_INDEX: usize = 6;

/// Entity index of a `CBaseEntity`: Windows reads the edict (`pev`, then the edict's
/// index); Linux reads the entity's own `CBaseHandle` (`m_RefEHandle`, at +8: the vtable
/// is at +0), whose low 14 bits are the index into the entity list.
pub(crate) unsafe fn entity_index(this: *const u8) -> Option<i32> {
    unsafe {
        let pev = *this.add(OFF_PEV).cast::<*const u8>();
        if pev.is_null() {
            None
        } else {
            Some(i32::from(*pev.add(OFF_EDICT_INDEX).cast::<i16>()))
        }
    }
}

/// Signature of the function. `??` bytes are the vtable displacement of the
/// `NetworkStateChanged` call and the offset of `m_nWaterLevel`, which differs between
/// game builds (0x2ab in the default build, 0x2b3 on the dev branch): the offset is read
/// from the match (see `learn_water_level_layout`), which also checks that the four
/// places that name it agree.
pub(crate) const SIG: &str = "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 \
                       48 83 EC 20 8B FA 0F B6 A9 ?? ?? 00 00 48 8B F1 \
                       38 91 ?? ?? 00 00 74 17 48 8B 01 48 8D 91 ?? ?? \
                       00 00 FF 90 ?? ?? 00 00 40 88 BE ?? ?? 00 00";

/// `CBaseEntity::UpdateWaterState` - used only to locate the engine's
/// `enginetrace` pointer: the `mov rcx,[rip+disp32]` inside it loads the
/// `IEngineTrace` object whose vtable slot 0 is
/// `GetPointContents(const Vector&, ...)`; the result is tested against
/// `MASK_WATER` (0x4030) to set water level 1, 2 and 3.
pub(crate) const UPDATE_WATER_STATE_SIG: &str = "\
    48 8B C4 53 48 83 EC 50 48 8B D9 0F 29 70 E8 48 81 C1 ?? ?? 00 00 48 C7 40 CC 00 00 00 00 \
    4C 8D 40 C8 C7 40 C8 00 00 00 00 48 8D 50 D8 C7 40 D8 00 00 00 3F 0F 57 F6 \
    48 C7 40 DC 00 00 00 3F E8 ?? ?? ?? ?? 33 D2 48 8B CB E8 ?? ?? ?? ?? 33 D2 48 8B CB \
    E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8D 54 24 20 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";

/// `83 F9 FF  74 ??  48 8B 35 <entity array>  8B C1  25 FF 3F 00 00
/// C1 E9 0E  48 C1 E0 05  39 4C 30 10`: the engine's "entity from handle"
/// idiom (entries are 32 bytes: pointer at +8, serial at +0x10). It occurs
/// many times, always with the same array global.
pub(crate) const ENTITY_ARRAY_SIG: &str =
    "83 F9 FF 74 ?? 48 8B 35 ?? ?? ?? ?? 8B C1 25 FF 3F 00 00 \
                                    C1 E9 0E 48 C1 E0 05 39 4C 30 10";

// ---- Fluid controllers (buoyancy) --------------------------------------
//
// How a map's water brush becomes buoyant, read from the 64-bit server.dll
// (world setup) and vphysics.dll:
//   obj = physenv->CreatePolyObjectStatic(collide, <water material>, ...)   // slot 8
//   physenv->CreateFluidController(obj, &fluidparams_t)                     // slot 11
// CreateFluidController (vphysics) takes the fluid's density from
// the object's *surface-property material* (so the object must use
// "water"), reads the surface plane (WORLD space, normal pointing out of
// the fluid), current velocity, damping (+0x1c), game data (+0x28) and
// contents (+0x34), and makes the object a trigger. Props that overlap it
// then get buoyancy and drag from vphysics. DestroyFluidController is
// slot 12 (SDK order; slots 8 and 11 were both confirmed).

/// Finds `mov rcx,[physenv]; lea r8,[..]; mov rdx,rsi; mov r9,[rcx];
/// call [r9+0x58]` in the world-setup code. The rip displacement of the
/// first instruction is the `physenv` global.
pub(crate) const PHYSENV_SIG: &str =
    "48 8B 0D ?? ?? ?? ?? 4C 8D 85 ?? ?? ?? ?? 48 8B D6 4C 8B 09 41 FF 51 58";

pub(crate) const CREATE_FLUID_SLOT: usize = 11;

pub(crate) const DESTROY_FLUID_SLOT: usize = 12;

/// `IPhysicsObject` vtable slots (confirmed in the world water setup:
/// `call [rbx+0xb8]`, `or ax,0x200`, `call [rbx+0xb0]`).
pub(crate) const OBJ_SET_CALLBACK_FLAGS_SLOT: usize = 0xb0 / 8;

pub(crate) const OBJ_GET_CALLBACK_FLAGS_SLOT: usize = 0xb8 / 8;

// ---- CCollisionEvent::FluidEndTouch(IPhysicsObject *, IPhysicsFluidController *) --
//
// A physics object remembers the fluids it is touching, and when it is
// destroyed vphysics calls FluidEndTouch for each of them, even for a
// fluid controller that was destroyed in the meantime (our DetachFluid).
// The splash code in there reads the dead controller and crashed the
// game: removing a water volume (or the prop in it) while a prop was
// inside. The hook recognises the controllers this module destroyed and
// does the rest of FluidEndTouch without them: it clears
// EFL_TOUCHING_FLUID and sends ENTITY_EVENT_WATER_UNTOUCH, no splash.
// (The signature covers the entity offsets and slots used here.)

pub(crate) const FLUID_END_TOUCH_SIG: &str = "48 89 5C 24 10 48 89 7C 24 18 41 56 48 83 EC 20 \
    FF 81 ?? ?? 00 00 4D 8B F0 48 8B DA 48 8B F9 48 85 D2 0F 84 ?? ?? ?? ?? 4D 85 C0 74 ?? \
    48 8B 02 48 8B CA 48 89 74 24 30 FF 90 88 00 00 00 48 8B F0 48 85 C0 74 ?? \
    48 8B D0 48 8B CF E8 ?? ?? ?? ?? 0F 2F 05 ?? ?? ?? ?? 72 ?? 4C 8B C6 48 8B D3 49 8B CE \
    E8 ?? ?? ?? ?? 81 A6 ?? ?? 00 00 FF FF F7 FF 49 8B CE 49 8B 06 48 8B 1E FF 50 30 \
    4C 63 C0 BA 01 00 00 00 48 8B CE FF 93 ?? ?? 00 00 FF 8F ?? ?? 00 00";

/// `IPhysicsObject::GetGameData` (the entity), vtable offset 0x88.
pub(crate) const OBJ_GET_GAME_DATA_SLOT: usize = 0x88 / 8;

/// `CBaseEntity::m_iEFlags` and its `EFL_TOUCHING_FLUID` bit.
pub(crate) const ENTITY_EFLAGS_OFFSET: usize = 0x190;

/// `CBaseEntity::OnEntityEvent(event`, data), vtable offset 0x2e8.
pub(crate) const ENTITY_EVENT_SLOT: usize = 0x2e8 / 8;

// ---- UTIL_BubbleTrail(const Vector &from, const Vector &to, int count) --
//
// Found in the 64-bit server.dll by signature (5 call sites; it is the
// only user of the bubble sprite index that takes a from/to pair). It asks
// for the water height above both end points with a downward MASK_WATER
// ray (the ray helper), clips the trail by that *depth* only, and
// sends the whole remaining segment to clients as one temp entity with
// `count` bubbles. Nothing limits the trail sideways, so a shot that
// leaves a volume through a side wall produces a trail as long as the
// bullet's flight. In a map the pool walls do that job; a volume has none.
//
// The hook clips the segment to the volume it is in (or enters) first and
// scales `count` by the kept fraction; everything else is left alone.

pub(crate) const BUBBLE_TRAIL_SIG: &str =
    "48 8B C4 48 89 58 08 48 89 70 10 57 48 81 EC E0 00 00 00 \
                                    F3 0F 10 49 08 41 8B F0 0F 29 70 E8 0F 28 D1 \
                                    F3 0F 58 15 ?? ?? ?? ?? 0F 29 78 D8 48 8B FA \
                                    44 0F 29 48 B8 48 8B D9 E8";

// ---- Explosions ------------------------------------------------------
//
// Several explosive entities choose between the normal explosion and the
// engine's water one (the "WaterSurfaceExplosion" effect: underwater
// sound, splash and wake, all made by the client from that one effect)
// by looking at their own m_nWaterLevel (byte at this+0x2ab), not at
// point contents:
//   * 0x5ae2f0  needs a water level of 2 or more ("deep enough")
//   * 0x5c9540 and 0x6df9d0  need any water level
// (+0x33c is the entity's absolute origin.) An entity whose origin is in a
// volume gets its water level raised to 2 before the original runs.

/// Per function: the prologue of each known build (tried in order).
/// Newer builds store rbx at [rsp+0x10] and reshuffle the pushes.
pub(crate) const EXPLOSION_TARGETS: [&[&str]; 3] = [
    &[
        "48 89 5C 24 08 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 02",
        "48 89 5C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 02",
    ],
    &[
        "48 8B C4 48 89 58 08 48 89 70 10 48 89 78 18 4C 89 70 20 55 48 8D 68 D8 48 81 EC 20 01 00 00 0F 29 70 E8 48 8B D9 8B 89 ?? 02 00 00",
        "48 8B C4 48 89 58 10 48 89 70 18 55 57 41 56 48 8D 68 C8 48 81 EC 20 01 00 00 0F 29 70 D8 48 8B D9 8B 89 ?? 02 00 00",
    ],
    &[
        "48 89 5C 24 08 48 89 7C 24 10 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 00",
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D 6C 24 A9 48 81 EC F0 00 00 00 80 B9 ?? 02 00 00 00",
    ],
];

pub(crate) const ENTITY_WATER_LEVEL_OFFSET: usize = 0x2ab;

pub(crate) const ENTITY_ORIGIN_OFFSET: usize = 0x33c;

/// Reads `m_nWaterLevel`'s offset out of a match of `SIG` (the four places that name it
/// must agree) and derives where the origin is.
pub(crate) unsafe fn learn_water_level_layout(at: *const u8) -> Result<usize, &'static str> {
    unsafe {
        let off = water_level_offset(&decode_match(at, SIG))?;
        WATER_LEVEL_OFFSET.store(off, Ordering::SeqCst);
        ORIGIN_OFFSET.store(off + ORIGIN_AFTER_WATER_LEVEL, Ordering::SeqCst);
        Ok(off)
    }
}

/// Reads `m_iEFlags`' offset and the `OnEntityEvent` vtable slot out of a match of
/// `FLUID_END_TOUCH_SIG`.
pub(crate) unsafe fn learn_fluid_layout(at: *const u8) -> Result<(), &'static str> {
    unsafe {
        let (flags, event_slot) = fluid_layout(&decode_match(at, FLUID_END_TOUCH_SIG))?;
        EFLAGS_OFFSET.store(flags, Ordering::SeqCst);
        EVENT_SLOT.store(event_slot, Ordering::SeqCst);
        Ok(())
    }
}

/// DispatchEffect(const char *name, const `CEffectData` &data).
/// Only its address is used (to send the water explosion effect), nothing is patched.
pub(crate) const DISPATCH_EFFECT_SIG: &str = "48 89 5C 24 08 57 48 83 EC 70 48 8B D9 48 8B FA \
                                       48 8D 4C 24 30 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 8B D7";

/// `CTEExplosion` fields: scale (f32), flags (u32), magnitude (i32).
pub(crate) const TE_SCALE_OFFSET: usize = 0x28;

pub(crate) const TE_FLAGS_OFFSET: usize = 0x2c;

pub(crate) const TE_MAGNITUDE_OFFSET: usize = 0x34;

/// True for a `CTEExplosion`: Windows compares the vtable (found through RTTI); Linux has
/// no RTTI for it and compares the temp entity's name, "Explosion".
pub(crate) unsafe fn is_te_explosion(this: *const u8) -> bool {
    unsafe {
        let vtable = TE_EXPLOSION_VTABLE.load(Ordering::Relaxed);
        vtable != 0 && this.cast::<usize>().read() == vtable
    }
}

/// Address of `CTEExplosion`'s vtable, found through RTTI only: the type
/// descriptor name, the complete object locator that names it, and the
/// pointer to that locator just before the vtable. No code bytes are
/// involved, so it does not depend on how the compiler laid out any
/// function.
pub(crate) unsafe fn te_explosion_vtable(base: *const u8) -> Result<usize, &'static str> {
    unsafe { rtti_vtable(base, b".?AVCTEExplosion@@\0") }
}

pub(crate) unsafe fn rtti_vtable(base: *const u8, name: &[u8]) -> Result<usize, &'static str> {
    unsafe {
        let rd32 = |off: usize| base.add(off).cast::<u32>().read_unaligned();
        let rd16 = |off: usize| base.add(off).cast::<u16>().read_unaligned();
        if rd16(0) != 0x5A4D {
            return Err("module has no MZ header");
        }
        let nt = rd32(0x3C) as usize;
        let preferred = base.add(nt + 24 + 24).cast::<u64>().read_unaligned() as usize;
        let nsec = rd16(nt + 6) as usize;
        let sections = nt + 24 + rd16(nt + 20) as usize;
        let mut data: Vec<(usize, &[u8])> = Vec::new(); // (rva, bytes) of non-code sections
        for i in 0..nsec {
            let sh = sections + i * 40;
            let vsize = rd32(sh + 8) as usize;
            let va = rd32(sh + 12) as usize;
            if rd32(sh + 36) & 0x2000_0000 == 0 && rd32(sh + 36) & 0x4000_0000 != 0 {
                data.push((va, std::slice::from_raw_parts(base.add(va), vsize)));
            }
        }
        let find = |needle: &[u8], align: usize| -> Vec<usize> {
            let mut out = Vec::new();
            for (va, bytes) in &data {
                let mut i = 0;
                while i + needle.len() <= bytes.len() {
                    if &bytes[i..i + needle.len()] == needle && (va + i) % align == 0 {
                        out.push(va + i);
                    }
                    i += 1;
                }
            }
            out
        };
        let names = find(name, 1);
        let Some(&name_rva) = names.first() else {
            return Err("RTTI type name not found");
        };
        let td_rva = (name_rva - 16) as u32;
        for at in find(&td_rva.to_le_bytes(), 4) {
            if at < 12 {
                continue;
            }
            let col = at - 12;
            // complete object locator: signature 1 (x64), offset 0
            if rd32(col) != 1 || rd32(col + 4) != 0 {
                continue;
            }
            // `vtable[-1]` holds its address: relocated, or preferred-base when only mapped.
            for target in [base as usize + col, preferred + col] {
                if let Some(p) = find(&target.to_le_bytes(), 8).into_iter().next() {
                    return Ok(base as usize + p + 8);
                }
            }
        }
        Err("CTEExplosion vtable not found")
    }
}

/// `CBaseTempEntity::Create(this`, filter, delay): every temp entity goes
/// through it (`CTempEnts::Explosion` jumps straight to it, so the vtable
/// slot is never used). Only `CTEExplosion` objects are looked at.
pub(crate) const TE_CREATE_SIG: &str =
    "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 \
                                 48 8B 05 ?? ?? ?? ?? 48 8B EA 0F 29 74 24 30";
