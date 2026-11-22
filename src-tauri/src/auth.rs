//! 本地账号：单人、单行（local_user id=1）。密码用 argon2id 哈希存本地 SQLite，
//! 绝不存明文；会话是否解锁是内存态，进程重启回到未解锁，需要重新登录。
//!
//! 注意：这是"防止别人随手打开就看到题库"的本地门禁，不是数据加密。
//! 真要防磁盘取证需要 SQLCipher（方案里的 P2），当前版本不做。

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand_core::OsRng;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{AuthProfileInput, AuthStatus};

fn hash_password(password: &str) -> Result<String, String> {
    if password.chars().count() < 6 {
        return Err("密码至少 6 位".into());
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| format!("密码哈希失败：{e}"))
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

fn validate_username(username: &str) -> Result<String, String> {
    let name = username.trim();
    if name.len() < 2 || name.len() > 32 {
        return Err("用户名需要 2–32 个字符".into());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("用户名不能有特殊字符".into());
    }
    Ok(name.to_string())
}

/// 本地用户只有一行。读取当前状态；没有账号时 has_user=false。
pub fn status(conn: &Connection, unlocked: bool) -> Result<AuthStatus, String> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "select username, display_name, bio, created_at from local_user where id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(match row {
        Some((username, display_name, bio, created_at)) => AuthStatus {
            has_user: true,
            unlocked,
            username,
            display_name,
            bio,
            created_at: Some(created_at),
        },
        None => AuthStatus {
            has_user: false,
            unlocked: false,
            username: String::new(),
            display_name: String::new(),
            bio: String::new(),
            created_at: None,
        },
    })
}

/// 首次设置账号。已有账号时拒绝，避免被覆盖导致永远登录不进去。
pub fn signup(
    conn: &Connection,
    username: &str,
    password: &str,
    display_name: &str,
) -> Result<AuthStatus, String> {
    let exists: i64 = conn
        .query_row("select count(*) from local_user where id = 1", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    if exists > 0 {
        return Err("本地账号已存在，请直接登录".into());
    }
    let username = validate_username(username)?;
    let hash = hash_password(password)?;
    let now = crate::text::now();
    let display = if display_name.trim().is_empty() {
        username.clone()
    } else {
        display_name.trim().to_string()
    };
    conn.execute(
        "insert into local_user (id, username, password_hash, display_name, bio, created_at, updated_at)
         values (1, ?1, ?2, ?3, '', ?4, ?4)",
        params![username, hash, display, now],
    )
    .map_err(|e| e.to_string())?;
    status(conn, true)
}

/// 登录校验。用户名/密码错误统一报同一句话，不透露是哪个错。
pub fn login(
    conn: &Connection,
    unlocked: &mut bool,
    username: &str,
    password: &str,
) -> Result<AuthStatus, String> {
    let row: Option<(String, String)> = conn
        .query_row(
            "select password_hash, display_name from local_user where id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (hash, _display) = row.ok_or_else(|| "还没有设置本地账号".to_string())?;
    let stored_username: String = conn
        .query_row("select username from local_user where id = 1", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    if stored_username != username.trim() || !verify_password(password, &hash) {
        return Err("用户名或密码不对".into());
    }
    *unlocked = true;
    status(conn, true)
}

pub fn logout(unlocked: &mut bool) {
    *unlocked = false;
}

/// 改密：必须校验旧密码。改完当前会话保持解锁。
pub fn change_password(
    conn: &Connection,
    unlocked: &mut bool,
    old_password: &str,
    new_password: &str,
) -> Result<(), String> {
    let hash: String = conn
        .query_row(
            "select password_hash from local_user where id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "还没有设置本地账号".to_string())?;
    if !verify_password(old_password, &hash) {
        return Err("旧密码不对".into());
    }
    if new_password == old_password {
        return Err("新密码不能和旧密码一样".into());
    }
    let new_hash = hash_password(new_password)?;
    let now = crate::text::now();
    conn.execute(
        "update local_user set password_hash = ?1, updated_at = ?2 where id = 1",
        params![new_hash, now],
    )
    .map_err(|e| e.to_string())?;
    *unlocked = true;
    Ok(())
}

/// 改昵称 / 签名。
pub fn update_profile(conn: &Connection, input: &AuthProfileInput) -> Result<AuthStatus, String> {
    let display = if input.display_name.trim().is_empty() {
        "我".to_string()
    } else {
        input.display_name.trim().to_string()
    };
    if input.bio.chars().count() > 200 {
        return Err("签名最多 200 字".into());
    }
    let now = crate::text::now();
    conn.execute(
        "update local_user set display_name = ?1, bio = ?2, updated_at = ?3 where id = 1",
        params![display, input.bio.trim(), now],
    )
    .map_err(|e| e.to_string())?;
    let unlocked = true;
    status(conn, unlocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_never_stores_plaintext() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(!hash.contains("correct horse"));
        assert!(hash.starts_with("$argon2"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn password_too_short_is_rejected() {
        assert!(hash_password("12345").is_err());
        assert!(hash_password("123456").is_ok());
    }
}
