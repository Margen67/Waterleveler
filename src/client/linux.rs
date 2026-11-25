// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;

/// client.so: the same function loads it with `lea r13,[rip+disp32]` (disp32 at byte 3),
/// then `mov rdi,[r13]; mov rax,[rdi]; call [rax]` (`GetPointContents`) and tests `MASK_WATER`.
pub(crate) const CLIENT_TRACE_SIG: &str = linux_sigs::CLIENT_TRACE_SIG;
