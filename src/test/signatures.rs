// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Margen67

use super::*;

#[test]
pub(crate) fn client_trace_signature_and_ray_helper_match_once_in_client_dll() {
    let Some(d) = client_dll() else {
        eprintln!("CLIENT_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let (_, _, _, text_raw, text_size, _) =
        pe.sections.iter().find(|s| s.0 == ".text").unwrap().clone();
    let text = &d[text_raw..text_raw + text_size];
    for sig in [CLIENT_TRACE_SIG, RAY_HELPER_SIG] {
        assert_eq!(find_all(text, &parse_sig(sig)).len(), 1, "{sig}");
    }
}

#[test]
pub(crate) fn signature_matcher_handles_wildcards() {
    let pat = parse_sig("01 ?? 03");
    assert_eq!(find_all(&[9, 1, 7, 3, 1, 8, 3, 1], &pat), vec![1, 4]);
    assert!(find_all(&[1, 2], &pat).is_empty());
}

/// RVA of the one place SIG matches in the file's executable sections.
pub(crate) fn sig_rva(d: &[u8], pe: &Pe) -> usize {
    let pat = parse_sig(SIG);
    let mut total = Vec::new();
    for (_, rva, _, raw, rawsz, chars) in &pe.sections {
        if chars & 0x2000_0000 != 0 {
            for off in find_all(&d[*raw..*raw + *rawsz], &pat) {
                total.push(rva + off);
            }
        }
    }
    assert_eq!(
        total.len(),
        1,
        "SetWaterLevel signature must match exactly once: {total:x?}"
    );
    total[0]
}

#[test]
pub(crate) fn signature_is_unique_in_server_dll() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let pat = parse_sig(SIG);
    let mut total = Vec::new();
    for (name, rva, _, raw, rawsz, chars) in &pe.sections {
        if chars & 0x2000_0000 != 0 {
            for off in find_all(&d[*raw..*raw + *rawsz], &pat) {
                total.push((name.clone(), rva + off));
            }
        }
    }
    assert_eq!(total.len(), 1, "{total:?}");
    assert_eq!(total[0].0, ".text");
}

#[test]
pub(crate) fn scan_image_finds_the_function() {
    let Some(d) = server_dll() else { return };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    let found = unsafe { scan_image(img.as_ptr()) }.expect("scan failed");
    assert_eq!(found as usize - img.as_ptr() as usize, sig_rva(&d, &pe));
}

#[test]
pub(crate) fn ray_helper_signature_gives_the_swept_offset() {
    let Some(d) = server_dll() else {
        eprintln!("SERVER_DLL_X64 not set; skipping");
        return;
    };
    let pe = parse_pe(&d);
    let img = map_image(&d, &pe);
    unsafe {
        let f = scan_sig(img.as_ptr(), RAY_HELPER_SIG).expect("ray helper must match exactly once");
        let off = ray_swept_offset_from(f);
        assert!(
            off == 0x41 || off == 0x49,
            "unexpected m_IsSwept offset {off:#x}"
        );
    }
}
