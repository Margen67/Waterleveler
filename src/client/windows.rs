// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

/// client.dll: `C_BaseEntity::UpdateWaterState` loads the client's `enginetrace`
/// the same way (`mov rcx,[rip+disp32]`, `GetPointContents`, test `MASK_WATER`). The
/// pattern is only the load and the test, so it does not depend on the function's
/// frame; disp32 is at byte 3 of the match.
pub(crate) const CLIENT_TRACE_SIG: &str =
    "48 8B 0D ?? ?? ?? ?? 48 8D 54 24 ?? 45 33 C0 48 8B 01 FF 10 A9 30 40 00 00";
