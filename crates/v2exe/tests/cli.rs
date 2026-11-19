use std::path::Path;
use std::process::Stdio;

use assert_cmd::Command;
use assert_cmd::cargo::{cargo_bin, cargo_bin_cmd};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOPICS: &str = include_str!("../../v2ex-rs/tests/fixtures/api1/topics.json");
const TOPIC: &str = include_str!("../../v2ex-rs/tests/fixtures/api1/topic.json");
const TOPIC_EMPTY: &str = include_str!("../../v2ex-rs/tests/fixtures/api1/topic_empty.json");
const TOPIC_RESTRICTED: &str =
    include_str!("../../v2ex-rs/tests/fixtures/api1/topic_restricted.json");
const REPLIES: &str = include_str!("../../v2ex-rs/tests/fixtures/api1/replies.json");
const NODES: &str = include_str!("../../v2ex-rs/tests/fixtures/api1/nodes.json");
const NODE_FEED: &str = include_str!("../../v2ex-rs/tests/fixtures/feed/node.xml");
const TAB_FEED: &str = include_str!("../../v2ex-rs/tests/fixtures/feed/tab.xml");

const USER_AGENT: &str = concat!("v2exe/", env!("CARGO_PKG_VERSION"));

/// Variables that would send test traffic elsewhere or switch colors on and off.
const CLEARED_ENV: [&str; 7] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "NO_COLOR",
];

const TOPIC_BLOCK: &str = "示例标题一
alice · Python · 2026-11-19 09:00 · 3 回复
https://www.v2ex.com/t/1001

第一段

#1 bob · 11-19 10:30
    @alice 同意

#2 erin · 11-19 11:00
    第二则
";

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

fn command(config_home: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("v2exe");
    for var in CLEARED_ENV {
        cmd.env_remove(var);
    }
    cmd.env("TZ", "Asia/Taipei")
        .env("XDG_CONFIG_HOME", config_home);
    cmd
}

fn finish(mut cmd: Command) -> Output {
    let out = cmd.output().unwrap();
    Output {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// Runs `v2exe --base-url <server> <args>` with an empty config directory.
fn run_against(server: &MockServer, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = command(home.path());
    cmd.args(["--base-url", &server.uri()]).args(args);
    finish(cmd)
}

fn serve(path_: &str, query: &[(&str, &str)], body: &str) -> Mock {
    let mut matcher = Mock::given(method("GET")).and(path(path_));
    for &(key, value) in query {
        matcher = matcher.and(query_param(key, value));
    }
    matcher.respond_with(ResponseTemplate::new(200).set_body_string(body))
}

fn row(id: u64, replies: u32, time: &str, author: &str, title: &str) -> String {
    format!("{id:>7}  {replies:>4}  {time}  {author:<15}  {title}\n")
}

fn hot_rows() -> String {
    row(1001, 3, "11-19 09:00", "alice", "[Python] 示例标题一")
        + &row(1003, 120, "11-19 11:05", "dave", "[问与答] 示例标题三")
}

#[tokio::test]
async fn hot_prints_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/topics/hot.json"))
        .and(header("user-agent", USER_AGENT))
        .respond_with(ResponseTemplate::new(200).set_body_string(TOPICS))
        .expect(1)
        .mount(&server)
        .await;

    let out = run_against(&server, &["hot"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, hot_rows());
}

#[tokio::test]
async fn hot_json() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;

    let out = run_against(&server, &["--json", "hot"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["id"], 1001);
    assert_eq!(v[0]["created"], "2026-11-19T01:00:00Z");
}

#[tokio::test]
async fn limit_truncates() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;

    let out = run_against(&server, &["hot", "--limit", "1"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1);
}

#[tokio::test]
async fn latest_tab_and_node_read_their_own_sources() {
    let server = MockServer::start().await;
    serve("/api/topics/latest.json", &[], TOPICS)
        .expect(1)
        .mount(&server)
        .await;
    serve("/feed/tab/tech.xml", &[], TAB_FEED)
        .expect(1)
        .mount(&server)
        .await;
    serve("/feed/python.xml", &[], NODE_FEED)
        .expect(1)
        .mount(&server)
        .await;

    let latest = run_against(&server, &["latest"]);
    let tab = run_against(&server, &["tab", "tech"]);
    let node = run_against(&server, &["node", "python"]);

    assert_eq!(latest.code, 0, "{}", latest.stderr);
    assert_eq!(latest.stdout, hot_rows());
    assert_eq!(tab.code, 0, "{}", tab.stderr);
    assert!(tab.stdout.contains("拼团"), "{}", tab.stdout);
    assert_eq!(node.code, 0, "{}", node.stderr);
    assert!(node.stdout.lines().count() >= 1);
}

#[tokio::test]
async fn topic_text() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1001")], TOPIC)
        .mount(&server)
        .await;
    serve("/api/replies/show.json", &[("topic_id", "1001")], REPLIES)
        .mount(&server)
        .await;

    let out = run_against(&server, &["topic", "1001"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, TOPIC_BLOCK);
}

#[tokio::test]
async fn topic_json_without_replies() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1001")], TOPIC)
        .mount(&server)
        .await;
    serve("/api/replies/show.json", &[], REPLIES)
        .expect(0)
        .mount(&server)
        .await;

    let out = run_against(
        &server,
        &[
            "--json",
            "topic",
            "https://www.v2ex.com/t/1001#reply2",
            "--no-replies",
        ],
    );

    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["topic"]["id"], 1001);
    assert!(v.get("replies").is_none());
}

#[tokio::test]
async fn title_only_topic_with_no_replies() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1004")], TOPIC_EMPTY)
        .mount(&server)
        .await;
    serve("/api/replies/show.json", &[("topic_id", "1004")], "[]")
        .mount(&server)
        .await;

    let text = run_against(&server, &["topic", "1004"]);
    let json = run_against(&server, &["--json", "topic", "1004"]);

    assert_eq!(text.code, 0, "{}", text.stderr);
    assert_eq!(
        text.stdout,
        "只有标题\ncarol · 问与答 · 2026-11-19 09:00 · 0 回复\nhttps://www.v2ex.com/t/1004\n"
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    let v: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(v["replies"], serde_json::json!([]));
}

#[tokio::test]
async fn restricted_topic_exits_5_before_fetching_replies() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "1002")], TOPIC_RESTRICTED)
        .mount(&server)
        .await;
    serve("/api/replies/show.json", &[], REPLIES)
        .expect(0)
        .mount(&server)
        .await;

    let out = run_against(&server, &["topic", "1002"]);

    assert_eq!(out.code, 5, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("需要登录"), "{}", out.stderr);
}

#[tokio::test]
async fn nodes_keyword_filters() {
    let server = MockServer::start().await;
    serve("/api/nodes/all.json", &[], NODES)
        .mount(&server)
        .await;

    let out = run_against(&server, &["nodes", "py"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("{:<20}  {:>6}  {}\n", "python", 16302, "Python")
    );
}

#[tokio::test]
async fn exit_codes_end_to_end() {
    let server = MockServer::start().await;
    serve("/api/topics/show.json", &[("id", "999")], "[]")
        .mount(&server)
        .await;
    let not_found = run_against(&server, &["topic", "999"]);
    assert_eq!(not_found.code, 3);
    assert!(not_found.stderr.contains("找不到"), "{}", not_found.stderr);
    assert!(
        not_found.stderr.starts_with("v2exe: "),
        "{}",
        not_found.stderr
    );

    let restricted = run_against(&server, &["node", "flamewar"]);
    assert_eq!(restricted.code, 5);

    let limited_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/topics/hot.json"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&limited_server)
        .await;
    assert_eq!(run_against(&limited_server, &["hot"]).code, 4);

    let home = tempfile::tempdir().unwrap();
    let mut unreachable = command(home.path());
    unreachable.args(["--base-url", "http://127.0.0.1:9", "hot"]);
    assert_eq!(finish(unreachable).code, 6);

    assert_eq!(run_against(&server, &["topic", "abc"]).code, 2);
    assert_eq!(
        run_against(&server, &["--proxy", "not a url", "hot"]).code,
        2
    );
}

#[tokio::test]
async fn missing_subcommand_prints_help_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = command(home.path());
    cmd.arg("--json");

    let out = finish(cmd);

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("Usage"), "{}", out.stderr);
}

#[tokio::test]
async fn version_flag_prints_the_version() {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = command(home.path());
    cmd.arg("--version");

    let out = finish(cmd);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        concat!("v2exe ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[tokio::test]
async fn config_file_supplies_base_url_and_flags_override() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join("v2exe");
    std::fs::create_dir_all(&config_dir).unwrap();

    std::fs::write(
        config_dir.join("config.toml"),
        format!("base_url = \"{}\"\n", server.uri()),
    )
    .unwrap();
    let mut from_config = command(home.path());
    from_config.arg("hot");
    let from_config = finish(from_config);
    assert_eq!(from_config.code, 0, "{}", from_config.stderr);
    assert_eq!(from_config.stdout, hot_rows());

    std::fs::write(
        config_dir.join("config.toml"),
        "base_url = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    let mut overridden = command(home.path());
    overridden.args(["--base-url", &server.uri(), "hot"]);
    let overridden = finish(overridden);
    assert_eq!(overridden.code, 0, "{}", overridden.stderr);
    assert_eq!(overridden.stdout, hot_rows());
}

#[tokio::test]
async fn malformed_config_exits_1() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join("v2exe");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    std::fs::write(&config, "base_url = ").unwrap();
    let mut cmd = command(home.path());
    cmd.arg("hot");

    let out = finish(cmd);

    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains(&config.display().to_string()),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("有误"), "{}", out.stderr);
    assert!(!out.stderr.contains("panicked"), "{}", out.stderr);
}

#[tokio::test]
async fn proxy_environment_variables_are_honoured() {
    let server = MockServer::start().await;
    serve("/api/topics/hot.json", &[], TOPICS)
        .mount(&server)
        .await;
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let home = tempfile::tempdir().unwrap();
    let hot = |no_proxy: Option<&str>| {
        let mut cmd = command(home.path());
        cmd.env("ALL_PROXY", format!("http://127.0.0.1:{closed_port}"))
            .args(["--base-url", &server.uri(), "hot"]);
        if let Some(hosts) = no_proxy {
            cmd.env("NO_PROXY", hosts);
        }
        finish(cmd)
    };

    let proxied = hot(None);
    let bypassed = hot(Some("127.0.0.1"));

    assert_eq!(proxied.code, 6, "{}", proxied.stderr);
    assert_eq!(bypassed.code, 0, "{}", bypassed.stderr);
    assert_eq!(bypassed.stdout, hot_rows());
}

#[tokio::test]
async fn zero_timeout_in_config_exits_1() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join("v2exe");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    std::fs::write(&config, "timeout = 0\n").unwrap();
    let mut cmd = command(home.path());
    cmd.arg("hot");

    let out = finish(cmd);

    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains(&config.display().to_string()),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("timeout"), "{}", out.stderr);
}

#[tokio::test]
async fn closed_pipe_exits_quietly() {
    let server = MockServer::start().await;
    serve("/api/nodes/all.json", &[], NODES)
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(cargo_bin!("v2exe"));
    for var in CLEARED_ENV {
        cmd.env_remove(var);
    }
    let mut child = cmd
        .env("TZ", "Asia/Taipei")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["--base-url", &server.uri(), "nodes"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());

    let out = child.wait_with_output().unwrap();

    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}
