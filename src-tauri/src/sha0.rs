//! SoftEther 的口令哈希：`Cedar/Account.c` 的 `HashPassword`——
//! `SHA0(密码字节 || 大写(用户名)字节)`。
//!
//! 这里的 SHA0 不是标准 SHA-0，也不能用任何现成 crate 替代：SoftEther 的
//! `Mayaqua/Encrypt.c` 里 `MY_SHA0_Transform` 是个混合变体——消息调度**不**做
//! 循环左移（同标准 SHA-0，那行 `W[t] = (1, ...)` 里的逗号运算符把旋转丢了），
//! 但轮内 `C = rol(30, B)` 保留了 SHA-1 的旋转。与标准 SHA-0、SHA-1 都不同，
//! 只能照抄；下面的测试向量全部取自 4.44 版 vpncmd 的实测输出。
//!
//! 用途只有一个：非 ASCII / 超长密码不能走 vpncmd 的交互提示（`PasswordPrompt`
//! 只收 0x20-0x7E 且缓冲 64 字节，见 `accounts.rs`），改为本地算好哈希直接写
//! `vpn_client.config` 的 `byte HashedPassword`。

/// SoftEther `HashPassword`：返回配置文件里 `byte HashedPassword` 的 base64 值。
///
/// 用户名按 SoftEther 的 `StrUpper` 规则只做大写 ASCII 转换（`ToUpper` 只动
/// a-z），与密码一样按 UTF-8 字节参与拼接。
pub fn hash_password_b64(username: &str, password: &str) -> String {
    let mut buf = Vec::with_capacity(password.len() + username.len());
    buf.extend_from_slice(password.as_bytes());
    buf.extend(username.bytes().map(|b| b.to_ascii_uppercase()));
    base64(&softether_sha0(&buf))
}

/// SoftEther 变体 SHA-0（见模块注释）。
fn softether_sha0(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

    // 填充：0x80 + 补零到 56 (mod 64) + 64 位大端比特长度
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            // 标准 SHA-0/SHA-1 在这里对 4 个字做 1 位旋转；SoftEther 不转
            w[i] = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => (d ^ (b & (c ^ d)), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (d & (b | c)), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(e)
                .wrapping_add(word)
                .wrapping_add(f)
                .wrapping_add(k);
            e = d;
            d = c;
            // 标准 SHA-0 不旋转 b；SoftEther 这里保留了 SHA-1 的 rol(30, B)
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// 标准 base64（带 `=` 补位），与客户端配置里 `byte` 值的编码一致。
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 0x3F] as char);
        out.push(TABLE[(n >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 0x3F] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 0x3F] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部向量取自 4.44 vpncmd 的 `AccountPasswordSet` 实测输出
    /// （`/PASSWORD:` 字面参数路径，字节忠实），用户名为账户里存的 Username。
    #[test]
    fn matches_vpncmd_password_hash() {
        let vectors: [(&str, &str, &str); 8] = [
            ("chenja", "", "FWWicS/yTW7CSNVaMEbTmwTOE8I="),
            ("u1", "Test1234", "djaHCniRSKhNs3RgNooZJdMx5VI="),
            ("chenja", "Test1234", "7Pl3uPusWFNFGKhN9Eyl8XaCR5c="),
            ("u1", "abcd", "2CR9pgc6xyF9NOWExOpxsouWwCk="),
            // 非 ASCII 密码：证明哈希按 UTF-8 字节算
            ("u1", "密码Test123", "AGfoLXEOlGdK9TmJxryNs0E2dfc="),
            ("u1", "Ab!@#$%^&*()_+ x", "h4V6RQAPTT7Amf7wWZ0ZLWWYKKs="),
            // 用户名只有 ASCII 参与大写转换
            ("AbC", "pw1", "bi4iZdPi3lFnJ0OZogv3QznKEcY="),
            // 70 字节密码跨两个 64 字节块
            ("u1", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "pG7HspeTNfhtDVPUWNWMf0FY5u8="),
        ];
        for (username, password, want) in vectors {
            assert_eq!(hash_password_b64(username, password), want, "{username} / {password}");
        }
    }

    #[test]
    fn username_case_does_not_matter() {
        assert_eq!(hash_password_b64("u1", "x"), hash_password_b64("U1", "x"));
    }
}
