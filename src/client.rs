// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

#[cfg(not(any(windows, test)))]
mod linux;
mod trace;
#[cfg(any(windows, test))]
mod windows;

#[cfg(not(any(windows, test)))]
pub(crate) use linux::*;
pub(crate) use trace::*;
#[cfg(any(windows, test))]
pub(crate) use windows::*;

use crate::*;
use std::ffi::c_int;

/// True when this DLL is the client module. With both features built in that is decided
/// from the file name (gmcl_*); a build with only one feature has no choice to make.
#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
#[cfg_attr(not(feature = "server"), allow(clippy::missing_const_for_fn))] // only const without the name check
pub(crate) unsafe fn running_as_client() -> bool {
    #[cfg(feature = "server")]
    unsafe {
        client_module::is_client_module()
    }
    #[cfg(not(feature = "server"))]
    true
}

/// Loaded as `gmcl_waterlevelercore`: the client's water. The Lua side gives it the
/// volumes (`AddVolume` etc.); its engine trace is
/// patched so the client sees and hears them like the server does.
#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
pub(crate) unsafe fn open_client(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        {
            let trace = match hook_state::install_client_contents() {
                Ok(note) => note,
                Err(why) => format!("trace hook failed: {why}"),
            };
            *HOOK_STATUS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = format!("client: {trace}");
            (a.createtable)(l, 0, 8);
            let t = base + 1;
            set_fn(l, t, c"AddVolume", l_add_volume);
            set_fn(l, t, c"RemoveVolume", l_remove_volume);
            set_fn(l, t, c"ClearVolumes", l_clear_volumes);
            set_fn(l, t, c"VolumeCount", l_volume_count);
            set_fn(l, t, c"PointInVolume", l_point_in_volume);
            set_fn(l, t, c"IsContentsHooked", l_is_contents_hooked);
            set_fn(l, t, c"ContentsHits", l_contents_hits);
            set_fn(l, t, c"TraceHits", l_trace_hits);
            set_fn(l, t, c"SetDebug", l_set_debug);
            set_fn(l, t, c"TakeDebugLog", l_take_debug_log);
            set_fn(l, t, c"HookStatus", l_hook_status);
            (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
            (a.settop)(l, base);
            0
        }
    }
}

#[cfg(all(
    any(windows, target_os = "linux"),
    target_arch = "x86_64",
    feature = "client"
))]
pub(crate) unsafe fn close_client(l: LuaState) -> c_int {
    unsafe {
        let a = api();
        let base = (a.gettop)(l);
        hook_state::uninstall_contents();
        CLIENT_ENTITY_LIST.store(0, std::sync::atomic::Ordering::SeqCst);
        volumes().clear();
        (a.pushnil)(l);
        (a.setfield)(l, LUA_GLOBALSINDEX, c"waterlevelercore".as_ptr());
        (a.settop)(l, base);
        0
    }
}
