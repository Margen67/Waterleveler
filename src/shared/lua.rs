// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use crate::*;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::OnceLock;

pub(crate) type LuaState = *mut c_void;

pub(crate) type LuaFn = unsafe extern "C-unwind" fn(LuaState) -> c_int;

pub(crate) const LUA_GLOBALSINDEX: c_int = -10002;

#[cfg(feature = "server")]
pub(crate) const LUA_MULTRET: c_int = -1;

pub(crate) const LUA_TNONE: c_int = -1;

pub(crate) const LUA_TNIL: c_int = 0;

pub(crate) const LUA_TSTRING: c_int = 4;

pub(crate) const LUA_TNUMBER: c_int = 3;

#[cfg(feature = "server")]
pub(crate) const LUA_TTABLE: c_int = 5;

#[cfg(feature = "server")]
pub(crate) const LUA_TUSERDATA: c_int = 7;

// ---------------------------------------------------------------------------
// Lua C API (resolved at runtime)
// ---------------------------------------------------------------------------

pub(crate) struct Api {
    pub(crate) gettop: unsafe extern "C-unwind" fn(LuaState) -> c_int,
    pub(crate) settop: unsafe extern "C-unwind" fn(LuaState, c_int),
    #[cfg(feature = "server")]
    pub(crate) pushvalue: unsafe extern "C-unwind" fn(LuaState, c_int),
    pub(crate) pushnil: unsafe extern "C-unwind" fn(LuaState),
    pub(crate) pushnumber: unsafe extern "C-unwind" fn(LuaState, f64),
    pub(crate) pushboolean: unsafe extern "C-unwind" fn(LuaState, c_int),
    pub(crate) pushstring: unsafe extern "C-unwind" fn(LuaState, *const c_char) -> *const c_char,
    pub(crate) pushcclosure: unsafe extern "C-unwind" fn(LuaState, LuaFn, c_int),
    pub(crate) createtable: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    #[cfg(feature = "server")]
    pub(crate) getfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    pub(crate) setfield: unsafe extern "C-unwind" fn(LuaState, c_int, *const c_char),
    #[cfg(feature = "server")]
    pub(crate) call: unsafe extern "C-unwind" fn(LuaState, c_int, c_int),
    #[cfg(feature = "server")]
    pub(crate) pcall: unsafe extern "C-unwind" fn(LuaState, c_int, c_int, c_int) -> c_int,
    pub(crate) tonumber: unsafe extern "C-unwind" fn(LuaState, c_int) -> f64,
    #[cfg(feature = "server")]
    pub(crate) toboolean: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    pub(crate) tolstring: unsafe extern "C-unwind" fn(LuaState, c_int, *mut usize) -> *const c_char,
    pub(crate) type_: unsafe extern "C-unwind" fn(LuaState, c_int) -> c_int,
    #[cfg(feature = "server")]
    pub(crate) touserdata: unsafe extern "C-unwind" fn(LuaState, c_int) -> *mut c_void,
}

pub(crate) static API: OnceLock<Api> = OnceLock::new();

#[inline]
pub(crate) fn api() -> &'static Api {
    // Only reachable after gmod13_open stored the table.
    API.get().expect("Lua API not loaded")
}

// The target of each transmute is the function pointer type of the `Api` field it fills.
#[allow(clippy::missing_transmute_annotations)]
pub(crate) unsafe fn load_api() -> Option<Api> {
    unsafe {
        macro_rules! load {
            ($name:literal) => {{
                let sym = find_symbol(CStr::from_bytes_with_nul_unchecked(
                    concat!($name, "\0").as_bytes(),
                ));
                if sym.is_null() {
                    eprintln!("[waterleveler] lua_shared has no symbol {}", $name);
                    return None;
                }
                std::mem::transmute::<*mut c_void, _>(sym)
            }};
        }
        Some(Api {
            gettop: load!("lua_gettop"),
            settop: load!("lua_settop"),
            #[cfg(feature = "server")]
            pushvalue: load!("lua_pushvalue"),
            pushnil: load!("lua_pushnil"),
            pushnumber: load!("lua_pushnumber"),
            pushboolean: load!("lua_pushboolean"),
            pushstring: load!("lua_pushstring"),
            pushcclosure: load!("lua_pushcclosure"),
            createtable: load!("lua_createtable"),
            #[cfg(feature = "server")]
            getfield: load!("lua_getfield"),
            setfield: load!("lua_setfield"),
            #[cfg(feature = "server")]
            call: load!("lua_call"),
            #[cfg(feature = "server")]
            pcall: load!("lua_pcall"),
            tonumber: load!("lua_tonumber"),
            #[cfg(feature = "server")]
            toboolean: load!("lua_toboolean"),
            tolstring: load!("lua_tolstring"),
            type_: load!("lua_type"),
            #[cfg(feature = "server")]
            touserdata: load!("lua_touserdata"),
        })
    }
}
