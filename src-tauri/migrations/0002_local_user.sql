-- 本地用户（单人客户端，只有一行 id=1）。 passwords 用 argon2id 哈希（PHC 字符串），绝不存明文。
create table if not exists local_user (
    id            integer primary key check (id = 1),
    username      text not null,
    password_hash text not null,
    display_name  text not null default '',
    bio           text not null default '',
    created_at    text not null,
    updated_at    text not null
);
