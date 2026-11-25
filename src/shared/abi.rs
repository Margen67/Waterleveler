// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

// The engine's calling convention: Windows x64 for the Windows server, System V for
// the Linux one. Under `cfg(test)` it is always Windows x64: the tests run the
// machine code of a Windows server.dll on whatever host runs them.
#[cfg(any(windows, test))]
macro_rules! engine_fn {
    ($($t:tt)*) => { unsafe extern "win64" fn $($t)* };
}

#[cfg(not(any(windows, test)))]
macro_rules! engine_fn {
    ($($t:tt)*) => { unsafe extern "C" fn $($t)* };
}

/// Defines an `unsafe fn` with the engine's calling convention.
macro_rules! engine_abi_item {
    ($(#[$m:meta])* $v:vis unsafe fn $name:ident ( $($args:tt)* ) $(-> $ret:ty)? $body:block) => {
        #[cfg(any(windows, test))]
        $(#[$m])* $v unsafe extern "win64" fn $name($($args)*) $(-> $ret)? $body
        #[cfg(not(any(windows, test)))]
        $(#[$m])* $v unsafe extern "C" fn $name($($args)*) $(-> $ret)? $body
    };
}
