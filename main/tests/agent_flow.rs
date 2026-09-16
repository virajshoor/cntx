//! End-to-end regression tests against a local mock provider.
//!
//! The mock speaks OpenAI-style chat completions and returns a scripted
//! sequence of tool requests; each test asserts the next request contains
//! the preceding tool result, so the shipped tool loop — not only helper
//! functions — is exercised. No real API charges are involved.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use cntx::app::{Runtime, RuntimeOptions};
use cntx::config::{AppConfig, ConfigStore, Effort, EndpointConfig, ProviderKind};
use cntx::permissions::Mode;
use cntx::providers::{ChatMessage, ChatRequest};
use cntx::sandbox::Sandbox;
use cntx::tools::{ToolHost, ToolLoopPromptContext};

/// A scripted HTTP server: each request is answered with the next canned
/// body, and the request line plus headers are recorded for assertions.
struct MockServer {
    addr: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    fn new(bodies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let canned = Arc::new(Mutex::new(bodies));
        let requests_clone = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some((request_line, headers, body)) = read_request(&mut stream) else {
                    continue;
                };
                let index = {
                    let mut seen = requests_clone.lock().unwrap();
                    seen.push(format!("{request_line}\n{headers}\n{body}"));
                    seen.len() - 1
                };
                let response_body = {
                    let responses = canned.lock().unwrap();
                    if index < responses.len() {
                        responses[index].clone()
                    } else {
                        String::new()
                    }
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Self { addr, requests }
    }

    fn request(&self, index: usize) -> String {
        self.requests.lock().unwrap()[index].clone()
    }

    fn endpoint(&self) -> EndpointConfig {
        let mut endpoint = EndpointConfig::new("mock", ProviderKind::OpenAiCompatible);
        endpoint.base_url = format!("http://{}", self.addr);
        endpoint.api_key = Some("mock-key".to_string());
        endpoint.default_model = Some("mock-model".to_string());
        endpoint
    }
}

/// Read one HTTP request: request line, headers, and Content-Length body.
fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, String)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut headers = String::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().ok()?;
        }
        headers.push_str(&line);
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    let body = String::from_utf8_lossy(&body).into_owned();
    Some((request_line, headers, body))
}

fn sse(content: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({ "choices": [{ "delta": { "content": content } }] })
    )
}

fn tool_call_text(name: &str, arguments: &str) -> String {
    format!("Working.\n<tool>{{\"name\":\"{name}\",\"arguments\":{arguments}}}</tool>\n")
}

/// Host adapter used by the tool loop in these tests.
struct TestHost {
    dry_run: bool,
    approve: bool,
    /// Fail `record_transcript` once this many entries exist, simulating a
    /// checkpoint/save failure.
    fail_record_after: Option<usize>,
    approvals: Arc<Mutex<Vec<String>>>,
    transcript: Arc<Mutex<Vec<String>>>,
}

impl Default for TestHost {
    fn default() -> Self {
        Self {
            dry_run: false,
            approve: true,
            fail_record_after: None,
            approvals: Arc::new(Mutex::new(Vec::new())),
            transcript: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl ToolHost for TestHost {
    fn dry_run(&self) -> bool {
        self.dry_run
    }

    fn approve(&mut self, action: &str) -> bool {
        self.approvals.lock().unwrap().push(action.to_string());
        self.approve
    }

    fn record_transcript(&mut self, role: &str, content: &str) -> anyhow::Result<()> {
        let mut transcript = self.transcript.lock().unwrap();
        if self
            .fail_record_after
            .is_some_and(|limit| transcript.len() >= limit)
        {
            anyhow::bail!("simulated checkpoint failure");
        }
        transcript.push(format!("{role}: {content}"));
        Ok(())
    }
}

fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    (temp, root)
}

fn plain_goal_context(session_id: Option<String>) -> ToolLoopPromptContext {
    ToolLoopPromptContext {
        history: Vec::new(),
        skill_prompt: None,
        effort: Effort::Medium,
        goal: None,
        session_id,
    }
}

#[test]
fn create_edit_execute_round_trip_through_mock_provider() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "write",
            r#"{"path":"nested/demo.txt","content":"hello from cntx"}"#,
        )),
        sse(&tool_call_text(
            "edit",
            r#"{"path":"nested/demo.txt","old_string":"hello","new_string":"goodbye"}"#,
        )),
        sse(&tool_call_text(
            "bash",
            r#"{"command":"cat nested/demo.txt","description":"read the file"}"#,
        )),
        sse("Created, edited, and verified nested/demo.txt."),
    ]);
    let (_guard, root) = workspace();
    let mut host = TestHost::default();
    let final_text = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "create then edit then verify nested/demo.txt",
            &Sandbox::new(Mode::Auto, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(Some("sess-1".to_string())),
            &mut host,
        ))
        .unwrap();

    // The next request after each tool call carries the preceding result.
    let second_request = server.request(1);
    assert!(second_request.contains("Tool result for 'write'"));
    assert!(second_request.contains("hello from cntx"));
    let third_request = server.request(2);
    assert!(third_request.contains("Tool result for 'edit'"));
    assert!(third_request.contains("Edited"));
    let fourth_request = server.request(3);
    assert!(
        fourth_request.contains("goodbye from cntx"),
        "bash result missing: {fourth_request}"
    );
    assert!(fourth_request.contains("Exit code: 0"));

    // The actual filesystem state matches the scripted edits.
    assert_eq!(
        std::fs::read_to_string(root.join("nested/demo.txt")).unwrap(),
        "goodbye from cntx"
    );
    assert_eq!(final_text, "Created, edited, and verified nested/demo.txt.");
    // The user prompt is part of the first request.
    assert!(server.request(0).contains("create then edit then verify"));
}

#[test]
fn denied_write_leaves_file_untouched_and_no_directories() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "write",
            r#"{"path":"newdir/out.txt","content":"should not exist"}"#,
        )),
        sse("Understood; I will not write the file."),
    ]);
    let (_guard, root) = workspace();
    let mut host = TestHost {
        approve: false,
        ..TestHost::default()
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "write newdir/out.txt",
            &Sandbox::new(Mode::Auto, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    let second_request = server.request(1);
    assert!(second_request.contains("User denied approval"));
    assert!(!root.join("newdir").exists());
    assert!(!root.join("newdir/out.txt").exists());
}

#[test]
fn manual_mode_asks_for_reads_and_denial_sends_no_contents() {
    let server = MockServer::new(vec![
        sse(&tool_call_text("read", r#"{"path":"secret.txt"}"#)),
        sse("Understood; I will not read it."),
    ]);
    let (_guard, root) = workspace();
    std::fs::write(root.join("secret.txt"), "classified contents").unwrap();
    let mut host = TestHost {
        approve: false,
        ..TestHost::default()
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "read secret.txt",
            &Sandbox::new(Mode::RequestPermission, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    // The read was asked, denied, and no contents reached the provider.
    let second_request = server.request(1);
    assert!(second_request.contains("User denied approval"));
    assert!(!second_request.contains("classified contents"));
}

#[test]
fn dry_run_blocks_mutations_and_shell() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "write",
            r#"{"path":"dry.txt","content":"x"}"#,
        )),
        sse(&tool_call_text(
            "bash",
            r#"{"command":"touch dry-ran.txt","description":"mutate"}"#,
        )),
        sse("Dry run complete; nothing changed."),
    ]);
    let (_guard, root) = workspace();
    let mut host = TestHost {
        dry_run: true,
        ..TestHost::default()
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "dry run",
            &Sandbox::new(Mode::Allow, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    let write_request = server.request(1);
    assert!(write_request.contains("Dry run"));
    let shell_request = server.request(2);
    assert!(shell_request.contains("Dry run"));
    assert!(!root.join("dry.txt").exists());
    assert!(!root.join("dry-ran.txt").exists());
}

#[test]
fn ambiguous_edit_fails_and_leaves_file_unchanged() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "edit",
            r#"{"path":"twice.txt","old_string":"dup","new_string":"new"}"#,
        )),
        sse("Understood."),
    ]);
    let (_guard, root) = workspace();
    std::fs::write(root.join("twice.txt"), "dup and dup again").unwrap();
    let mut host = TestHost::default();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "edit twice.txt",
            &Sandbox::new(Mode::Allow, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    let second_request = server.request(1);
    assert!(second_request.contains("exactly once"));
    assert_eq!(
        std::fs::read_to_string(root.join("twice.txt")).unwrap(),
        "dup and dup again"
    );
}

#[test]
fn large_command_output_finishes_without_deadlock_and_is_bounded() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "bash",
            r#"{"command":"head -c 2097152 /dev/zero | tr '\\0' 'x'","description":"large output"}"#,
        )),
        sse("Done."),
    ]);
    let (_guard, root) = workspace();
    let mut host = TestHost::default();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "big output",
            &Sandbox::new(Mode::Allow, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    let request = server.request(1);
    // Bounded: at most the read cap plus markers.
    assert!(
        request.len() < 32_000,
        "unbounded tool result: {}",
        request.len()
    );
    assert!(request.contains('x'));
}

#[test]
fn timeout_terminates_parent_and_child_processes() {
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "bash",
            // The command records the exact PIDs of its own children in the
            // workspace; the test checks those PIDs only, never a global
            // process-name scan that would count unrelated processes.
            r#"{"command":"sleep 30 & echo $! > timeout-pids.txt; sleep 30 & echo $! >> timeout-pids.txt; wait","description":"hangs","timeout_secs":1}"#,
        )),
        sse("Recovered."),
    ]);
    let (_guard, root) = workspace();
    let mut host = TestHost::default();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "timeout case",
            &Sandbox::new(Mode::Allow, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ))
        .unwrap();
    assert!(server.request(1).contains("timed out"));
    // Only this test's own child PIDs are checked: a global process-name
    // assertion counts unrelated processes on the machine.
    let pids = std::fs::read_to_string(root.join("timeout-pids.txt")).unwrap();
    let leftovers: Vec<&str> = pids
        .lines()
        .map(str::trim)
        .filter(|pid| !pid.is_empty())
        .filter(|pid| {
            std::process::Command::new("ps")
                .arg("-p")
                .arg(pid)
                .output()
                .map(|output| output.status.success())
                .unwrap_or(true)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover child processes: {leftovers:?}"
    );
}

/// A runtime wired to a local mock provider, used by goal/persistence
/// regression tests that exercise the shipped runtime loop.
fn mock_runtime(root: &std::path::Path, endpoint: EndpointConfig) -> Runtime {
    let mut endpoints = std::collections::BTreeMap::new();
    let name = endpoint.name.clone();
    endpoints.insert(name.clone(), endpoint);
    let config = AppConfig {
        primary_endpoint: Some(name),
        endpoints,
        ..AppConfig::default()
    };
    Runtime::new(
        config,
        ConfigStore::from_root(root),
        RuntimeOptions {
            endpoint_override: None,
            model_override: None,
            mode: Mode::Allow,
            effort: Effort::Medium,
            apply: false,
            dry_run: false,
            sandbox: Sandbox::new(Mode::Allow, root.to_path_buf(), Vec::new()),
            tool_use: true,
        },
    )
    .unwrap()
}

#[test]
fn goal_updates_validate_and_persist() {
    // A goal run against the mock: real tool work first, then a rejected
    // completion with unreferenced evidence, then a completion whose evidence
    // matches the recorded tool results. The goal state machine, evidence
    // validation, persistence, and reload are all covered.
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "write",
            r#"{"path":"goal.txt","content":"milestone"}"#,
        )),
        sse(&tool_call_text(
            "bash",
            r#"{"command":"cat goal.txt","description":"verify the file"}"#,
        )),
        sse(&tool_call_text(
            "goal_update",
            r#"{"status":"completed","evidence":"everything looks good"}"#,
        )),
        sse(&tool_call_text(
            "goal_update",
            r#"{"status":"completed","progress":"done","evidence":"cat goal.txt returned milestone"}"#,
        )),
    ]);
    let (_guard, root) = workspace();
    let mut runtime = mock_runtime(&root, server.endpoint());

    let message = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.start_goal("verify goal machinery"))
        .unwrap();
    assert!(message.contains("completed"), "{message}");

    // The objective is injected into the first request of the run, and each
    // request carries the preceding tool result.
    assert!(server.request(0).contains("verify goal machinery"));
    assert!(server.request(1).contains("Written"));
    assert!(server.request(2).contains("milestone"));
    // Completion evidence that references nothing recorded in this session
    // is rejected and fed back to the model.
    assert!(server
        .request(3)
        .contains("completion evidence must reference"));
    // The real work happened on disk and in the transcript.
    assert_eq!(
        std::fs::read_to_string(root.join("goal.txt")).unwrap(),
        "milestone"
    );

    let goal = runtime.session.goal.as_ref().unwrap();
    assert_eq!(goal.status, "completed");
    assert_eq!(goal.progress, "done");
    assert_eq!(goal.evidence, "cat goal.txt returned milestone");
    assert_eq!(goal.max_steps, 50);
    // Every provider turn counted toward the step budget.
    assert_eq!(goal.steps_used, 4);

    // The completed goal and transcript survive a session reload.
    let reloaded = cntx::sessions::SessionStore::new(&runtime.store)
        .load(&runtime.session.id)
        .unwrap();
    let goal = reloaded.goal.as_ref().unwrap();
    assert_eq!(goal.status, "completed");
    assert_eq!(goal.evidence, "cat goal.txt returned milestone");

    // User controls through the C machine: pause and cancel need no model
    // call; replacement is allowed only after cancel/completion.
    runtime.session.goal = Some(cntx::sessions::GoalState::new("second objective"));
    runtime.save_session().unwrap();
    assert_eq!(runtime.session.goal.as_ref().unwrap().status, "active");
    // Replacing an active goal is rejected before any model call.
    assert!(tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.start_goal("must be rejected"))
        .is_err());
    assert_eq!(
        runtime.session.goal.as_ref().unwrap().objective,
        "second objective"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 4);
    runtime.pause_goal().unwrap();
    assert_eq!(runtime.session.goal.as_ref().unwrap().status, "paused");
    runtime.cancel_goal().unwrap();
    assert_eq!(runtime.session.goal.as_ref().unwrap().status, "cancelled");
    // Replacing a cancelled goal is allowed.
    runtime.session.goal = Some(cntx::sessions::GoalState::new("third objective"));
    assert_eq!(runtime.session.goal.as_ref().unwrap().status, "active");
}

#[test]
fn goal_cap_pauses_without_completion_or_extra_requests() {
    // An exhausted step budget pauses the goal without another paid request
    // and without marking the goal complete.
    let server = MockServer::new(Vec::new());
    let (_guard, root) = workspace();
    let mut runtime = mock_runtime(&root, server.endpoint());
    let mut goal = cntx::sessions::GoalState::new("bounded work");
    goal.steps_used = 3;
    goal.max_steps = 3;
    runtime.session.goal = Some(goal);
    runtime.save_session().unwrap();

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.run_prompt("continue the goal"))
        .unwrap();

    // The budget check stops the turn before any provider request.
    assert!(server.requests.lock().unwrap().is_empty());
    let goal = runtime.session.goal.as_ref().unwrap();
    assert_eq!(goal.status, "paused");
    // The pause is persisted, not just in-memory state.
    let reloaded = cntx::sessions::SessionStore::new(&runtime.store)
        .load(&runtime.session.id)
        .unwrap();
    assert_eq!(reloaded.goal.as_ref().unwrap().status, "paused");
}

#[test]
fn goal_completion_stops_the_next_tool_in_the_same_response() {
    // goal_update(completed) followed by a write in one response: the loop
    // must stop at the completion and never execute the later tool.
    let server = MockServer::new(vec![sse(
        "Working.\n<tool>{\"name\":\"goal_update\",\"arguments\":{\"status\":\"completed\",\"evidence\":\"no executable check applies; nothing ran yet\"}}</tool>\n<tool>{\"name\":\"write\",\"arguments\":{\"path\":\"after-complete.txt\",\"content\":\"must not exist\"}}</tool>\n",
    )]);
    let (_guard, root) = workspace();
    let mut runtime = mock_runtime(&root, server.endpoint());

    let message = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.start_goal("finish quickly"))
        .unwrap();
    assert!(message.contains("completed"), "{message}");
    assert_eq!(runtime.session.goal.as_ref().unwrap().status, "completed");
    assert!(!root.join("after-complete.txt").exists());
}

#[test]
fn goal_denial_pauses_and_keeps_the_goal_resumable() {
    // Nonterminal stdin denies approval, which must pause the goal (not
    // complete or cancel it) and leave no side effects. Resuming then works
    // through the same loop with a fresh bounded budget.
    let server = MockServer::new(vec![
        sse(&tool_call_text(
            "write",
            r#"{"path":"denied.txt","content":"nope"}"#,
        )),
        sse(&tool_call_text(
            "goal_update",
            r#"{"status":"active","progress":"resumed and working"}"#,
        )),
        sse(&tool_call_text(
            "goal_update",
            r#"{"status":"completed","evidence":"user denied the write; no executable check applies"}"#,
        )),
    ]);
    let (_guard, root) = workspace();
    let mut runtime = mock_runtime(&root, server.endpoint());
    // auto-approve asks before writes; the test harness has no terminal.
    runtime.mode = Mode::Auto;
    runtime.sandbox = Sandbox::new(Mode::Auto, root.clone(), Vec::new());

    let message = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.start_goal("write a file"))
        .unwrap();
    assert!(message.contains("paused"), "{message}");
    let goal = runtime.session.goal.as_ref().unwrap();
    assert_eq!(goal.status, "paused");
    assert!(!root.join("denied.txt").exists());

    // Resuming grants a fresh bounded batch; the objective is preserved and
    // the loop continues through the same tool path.
    let message = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.resume_goal())
        .unwrap();
    assert!(message.contains("completed"), "{message}");
    let goal = runtime.session.goal.as_ref().unwrap();
    assert_eq!(goal.objective, "write a file");
    assert_eq!(goal.steps_used, 2);
    assert_eq!(goal.status, "completed");
}

#[test]
fn checkpoint_failure_stops_before_the_next_side_effect() {
    // When persisting a tool result fails, the loop must stop before
    // executing the next tool in the same response.
    let server = MockServer::new(vec![sse(
        "Working.\n<tool>{\"name\":\"write\",\"arguments\":{\"path\":\"first.txt\",\"content\":\"ok\"}}</tool>\n<tool>{\"name\":\"bash\",\"arguments\":{\"command\":\"touch side-effect.txt\",\"description\":\"mutate\"}}</tool>\n",
    )]);
    let (_guard, root) = workspace();
    let mut host = TestHost {
        fail_record_after: Some(1),
        ..TestHost::default()
    };
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cntx::tools::run_tool_loop(
            "two actions",
            &Sandbox::new(Mode::Allow, root.clone(), Vec::new()),
            &root,
            &server.endpoint(),
            "mock-model",
            plain_goal_context(None),
            &mut host,
        ));
    assert!(result.is_err(), "checkpoint failure must surface an error");
    // The first write happened and was recorded; the bash call after the
    // failed checkpoint never ran.
    assert_eq!(
        std::fs::read_to_string(root.join("first.txt")).unwrap(),
        "ok"
    );
    assert!(!root.join("side-effect.txt").exists());
}

#[test]
fn compaction_preserves_session_id_summary_and_goal_in_follow_up() {
    // `/compact` semantics: the session id is stable, older messages are
    // summarized through the provider, and the summary plus goal reach the
    // next request.
    let server = MockServer::new(vec![
        sse("Summary: the user asked to ship the widget feature."),
        sse("Summary: the assistant created the widget file."),
        sse("Follow-up done."),
    ]);
    let (_guard, root) = workspace();
    let mut runtime = mock_runtime(&root, server.endpoint());
    runtime.config.routing.input_token_budget = 2_000;
    runtime
        .session
        .push("user", "first request about the widget");
    runtime.session.push("assistant", "created the widget file");
    runtime
        .session
        .push("user", "second request about the gadget");
    runtime.session.push("assistant", "updated the gadget");
    runtime.session.push("user", "latest turn");
    runtime.session.goal = Some(cntx::sessions::GoalState::new("ship the widget feature"));
    runtime.save_session().unwrap();
    let session_id = runtime.session.id.clone();

    let message = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.compact_session())
        .unwrap();
    assert!(message.contains("compacted 2 messages"), "{message}");
    assert_eq!(runtime.session.id, session_id);
    assert_eq!(runtime.session.context_start_index, 2);
    let summary = runtime.session.summary.as_ref().unwrap();
    assert!(summary.contains("widget file"));

    // The follow-up request carries the summary, the preserved recent turns,
    // and the goal objective.
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(runtime.run_prompt("follow up on the widget"))
        .unwrap();
    let follow_up = server.request(2);
    assert!(follow_up.contains("Previous conversation summary"));
    assert!(follow_up.contains("widget file"));
    assert!(follow_up.contains("ship the widget feature"));
    assert!(follow_up.contains("follow up on the widget"));
    // The session id is still unchanged after the follow-up turn.
    assert_eq!(runtime.session.id, session_id);
}

/// The three Go protocols: correct API path, auth, client identity, session
/// header, and streamed text parsing, verified against a local mock.
#[test]
fn go_protocols_route_correctly_with_auth_and_session_headers() {
    // chat completions (GLM/Kimi/DeepSeek families)
    let chat = MockServer::new(vec![sse("chat ok")]);
    let request = ChatRequest {
        model: "opencode-go/glm-5.3-flash".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        max_tokens: Some(64),
        session_id: Some("go-session-1".to_string()),
    };
    let text = stream_once(chat.endpoint(), request, true);
    assert_eq!(text, "chat ok");
    let first_request = chat.request(0).to_lowercase();
    assert!(
        first_request.starts_with("post /chat/completions "),
        "{first_request}"
    );
    assert!(first_request.contains("authorization: bearer mock-key"));
    assert!(first_request.contains("user-agent: cntx/"));
    assert!(first_request.contains("x-opencode-session: go-session-1"));

    // messages protocol (MiniMax/Qwen families)
    let messages = MockServer::new(vec![format!(
        "data: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({ "type": "content_block_delta", "delta": { "text": "messages ok" } })
    )]);
    let request = ChatRequest {
        model: "minimax-m3".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        max_tokens: Some(64),
        session_id: Some("go-session-2".to_string()),
    };
    let text = stream_once(messages.endpoint(), request, true);
    assert_eq!(text, "messages ok");
    let request_line = messages.request(0).to_lowercase();
    assert!(
        request_line.starts_with("post /messages "),
        "{request_line}"
    );
    assert!(request_line.contains("authorization: bearer mock-key"));
    assert!(request_line.contains("x-opencode-session: go-session-2"));

    // responses protocol (GPT/Grok/Muse families)
    let responses = MockServer::new(vec![format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({ "type": "response.output_text.delta", "delta": "responses ok" }),
        serde_json::json!({ "type": "response.completed" })
    )]);
    let request = ChatRequest {
        model: "gpt-5.6-luna".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        max_tokens: Some(64),
        session_id: Some("go-session-3".to_string()),
    };
    let text = stream_once(responses.endpoint(), request, true);
    assert_eq!(text, "responses ok");
    let request_line = responses.request(0).to_lowercase();
    assert!(
        request_line.starts_with("post /responses "),
        "{request_line}"
    );
    assert!(request_line.contains("authorization: bearer mock-key"));
    assert!(request_line.contains("x-opencode-session: go-session-3"));
}

#[test]
fn go_inference_resolves_the_preset_stored_key_not_other_providers() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::from_root(temp.path());
    cntx::api_keys::ensure_secrets_file(&store).unwrap();
    cntx::api_keys::add(&store, "openai", "sk-openai-key").unwrap();
    let mut endpoint = EndpointConfig::new("opencode-go", ProviderKind::OpenAiCompatible);
    endpoint.base_url = "https://opencode.ai/zen/go/v1".to_string();
    endpoint
        .metadata
        .insert("preset".to_string(), serde_json::Value::from("opencode-go"));
    // No opencode-go key stored: must not fall back to the OpenAI key.
    assert!(cntx::api_keys::resolve_for_provider(&store, &endpoint).is_none());
    // The documented preset environment variable resolves for the endpoint.
    endpoint.api_key_env = Some("OPENCODE_GO_API_KEY".to_string());
    // Safety: only set a fake key for this test; never print it.
    std::env::set_var("OPENCODE_GO_API_KEY", "oc-go-env-key");
    assert_eq!(
        cntx::api_keys::resolve_for_provider(&store, &endpoint).as_deref(),
        Some("oc-go-env-key")
    );
    std::env::remove_var("OPENCODE_GO_API_KEY");
}

fn stream_once(endpoint: EndpointConfig, request: ChatRequest, go: bool) -> String {
    let mut endpoint = endpoint;
    if go {
        endpoint
            .metadata
            .entry("preset".to_string())
            .or_insert_with(|| serde_json::Value::from("opencode-go"));
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut text = String::new();
        cntx::providers::adapter_for(cntx::config::ProviderKind::OpenAiCompatible)
            .stream_chat(&endpoint, request, &mut |delta| text.push_str(&delta))
            .await
            .unwrap();
        text
    })
}
