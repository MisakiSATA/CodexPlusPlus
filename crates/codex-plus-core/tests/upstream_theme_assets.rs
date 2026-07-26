use sha2::{Digest, Sha256};

/// 校验前统一把 CRLF 归一化为 LF：这些资产都是文本文件，git 检出时
/// Windows 与 Linux 的行尾不同，直接按磁盘字节哈希会随平台漂移。
fn assert_sha256(relative_path: &str, expected: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative_path);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "failed to read upstream theme asset {}: {error}",
            path.display()
        )
    });
    let normalized = normalize_line_endings(&bytes);
    let actual = format!("{:X}", Sha256::digest(normalized));
    assert_eq!(actual, expected, "upstream asset changed: {relative_path}");
}

fn normalize_line_endings(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        normalized.push(bytes[index]);
        index += 1;
    }
    normalized
}

#[test]
fn bundled_target_renderers_and_styles_remain_byte_exact() {
    for (path, hash) in [
        (
            "assets/inject/upstream/dream-skin/windows/renderer-inject.js",
            "0BFB5F66A0323BF1392B42033E66904DE3EC4BFC8A5BA297F2BB92A4A6740A34",
        ),
        (
            "assets/inject/upstream/dream-skin/windows/dream-skin.css",
            "926ADA0A750A0EC3BE68B4B8F1E5DCEF5D58A85F3619B2B033856D4DF216EF7B",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/renderer-inject.js",
            "9ADAB4655C54740C2FCBBD5B2555AACDB659982B855706399B0F5367914511B3",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/dream-skin.css",
            "EC3C3BC5F6E10E20A3F2307796BD1E1350E80E5D23D37318EE5468833C95A6DF",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/renderer-inject.js",
            "0BFB5F66A0323BF1392B42033E66904DE3EC4BFC8A5BA297F2BB92A4A6740A34",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/dream-skin.css",
            "482A60AF98DD6B460BF624C56918C5B57F9CCD5B55E52FA46D486F7D65259D9A",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/renderer-inject.js",
            "4E2A74A337B4AB5EE12FE307565C1E6A309FDA8911DFA75C4B9BE8849C7BFF3C",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/dream-skin.css",
            "5E149E9A13985961C5F3125296178ACB2ABF0B528974F1E616AA625970430562",
        ),
        (
            "assets/inject/upstream/snow-skin/renderer-inject.js",
            "0FCDFF4AECD03EAB2CA4EE923CCD20CB97EB5460F7C9F07351A2003FFA76E6FA",
        ),
        (
            "assets/inject/upstream/snow-skin/dream-skin.css",
            "0AF2D20FBE3E3DD13F0BE7F1E5A90366E1501084827B22C1D4815A421BFCE823",
        ),
        (
            "assets/inject/upstream/glass-vision/renderer-inject.js",
            "D14943E95DB62DB81BF29D9CF14FCAF1DD1EA9A9625245C020865127EEA295A2",
        ),
        (
            "assets/inject/upstream/glass-vision/glass-vision.css",
            "4C37C53544EE4F1CD93BA5D0DC3E174B05D4CB84EC9A436295D11D19F0BB04F1",
        ),
    ] {
        assert_sha256(path, hash);
    }
}

#[test]
fn bundled_skin_pack_theme_files_remain_byte_exact() {
    for (path, hash) in [
        (
            "caishen-lite",
            "379CB601522E7A5C2FC906E3D8BD5C7C64385FBD2A428798F8D169DEB7026F2E",
        ),
        (
            "caishen-max",
            "0CFF815EC9582B88ECF4AD9E7562D7DFF32904FA8F09338811397960D62FD7D4",
        ),
        (
            "caishen-readable",
            "EB1FFD4F2F49137B4AEDDBED435513D42685C8ED9E97DF644C693FD7859CC62D",
        ),
        (
            "export-night",
            "C312329AABEE84B9A8443B08D4DB64863EC49DEEA3C7F7C942B57E4391B87B59",
        ),
        (
            "global-founder-bright",
            "EAFB018494225ABABD83AE0E7B940E3F565232CF8F30C53AAFA63E7652178810",
        ),
        (
            "mythic-guardian-noir",
            "2A57716D0161F7405D713912BCD0CD329038657518537F3EFDB5F7EE53DBAE3D",
        ),
    ] {
        assert_sha256(
            &format!("assets/inject/upstream/skin-packs/packs/{path}/theme.json"),
            hash,
        );
    }
}
