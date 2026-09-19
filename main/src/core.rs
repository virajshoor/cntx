//! Safe Rust wrapper around the C core ABI (`csrc/`).
//!
//! All `unsafe` FFI calls live in this module. Everything else delegates
//! migrated decisions here. The wrapper validates inputs (nulls, lengths,
//! UTF-8, NUL bytes) and translates C error codes into the existing error
//! handling. C never retains pointers to host memory: buffers are passed
//! with explicit lengths and freed by the caller that owns them.

use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;
use std::time::Duration;

/// Mode codes matching `cntx_mode_t` in `csrc/cntx.h`.
pub const MODE_AUTO_APPROVE: i32 = 0;
pub const MODE_COUNSEL: i32 = 1;
pub const MODE_ALL_APPROVE: i32 = 2;
pub const MODE_MANUAL_APPROVE: i32 = 3;
pub const MODE_FILE_ONLY: i32 = 4;
pub const MODE_PLAN: i32 = 5;

/// Operation codes matching `cntx_op_t`.
pub const OP_READ: i32 = 0;
pub const OP_WRITE: i32 = 1;
pub const OP_SHELL: i32 = 2;
pub const OP_NETWORK: i32 = 3;

/// Decision codes matching `cntx_decision_t`.
pub const DECISION_ALLOW: i32 = 0;
pub const DECISION_ASK: i32 = 1;
pub const DECISION_DENY: i32 = 2;

/// Goal status codes matching `cntx_goal_status_t`.
pub const GOAL_NONE: i32 = 0;
pub const GOAL_ACTIVE: i32 = 1;
pub const GOAL_PAUSED: i32 = 2;
pub const GOAL_BLOCKED: i32 = 3;
pub const GOAL_COMPLETED: i32 = 4;
pub const GOAL_CANCELLED: i32 = 5;

/// Goal event codes matching `cntx_goal_event_t`.
pub const GOAL_EVENT_START: i32 = 0;
pub const GOAL_EVENT_PAUSE: i32 = 1;
pub const GOAL_EVENT_RESUME: i32 = 2;
pub const GOAL_EVENT_CANCEL: i32 = 3;
pub const GOAL_EVENT_COMPLETE: i32 = 4;
pub const GOAL_EVENT_BLOCK: i32 = 5;
pub const GOAL_EVENT_STEP_LIMIT: i32 = 6;
pub const GOAL_EVENT_PROVIDER_FAILURE: i32 = 7;

/// Go protocol codes matching `cntx_go_protocol_t`.
pub const GO_PROTOCOL_CHAT: i32 = 0;
pub const GO_PROTOCOL_MESSAGES: i32 = 1;
pub const GO_PROTOCOL_RESPONSES: i32 = 2;
pub const GO_PROTOCOL_UNKNOWN: i32 = 3;

/// Counsel task codes matching `cntx_task_t`.
pub const TASK_EVALUATE: i32 = 0;
pub const TASK_SMALL_CHANGE: i32 = 1;
pub const TASK_REFACTOR: i32 = 2;

#[repr(C)]
struct ModelCandidate {
    id: *const c_char,
    created: i64,
    rank: i32,
}

#[link(name = "cntxcore")]
extern "C" {
    fn cntx_permission_decide(mode: c_int, operation: c_int) -> c_int;
    fn cntx_mode_canonical_name(mode: c_int) -> *const c_char;
    fn cntx_mode_description(mode: c_int) -> *const c_char;
    fn cntx_mode_parse(name: *const c_char, out_mode: *mut c_int) -> c_int;
    fn cntx_mode_next(mode: c_int) -> c_int;

    fn cntx_tool_validate(
        tool_name: *const c_char,
        keys: *const *const c_char,
        values: *const *const c_char,
        count: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;

    fn cntx_file_read(
        path: *const c_char,
        offset: u64,
        buf: *mut c_char,
        buf_len: usize,
        written: *mut usize,
        truncated: *mut c_int,
    ) -> c_int;
    fn cntx_file_write(path: *const c_char, content: *const c_char, content_len: usize) -> c_int;
    fn cntx_file_edit(
        path: *const c_char,
        old_text: *const c_char,
        new_text: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cntx_command_run(
        command: *const c_char,
        cwd: *const c_char,
        timeout_ms: u64,
        cancel: *const c_int,
        stdout_buf: *mut c_char,
        stdout_len: usize,
        stderr_buf: *mut c_char,
        stderr_len: usize,
        exit_code: *mut c_int,
        timed_out: *mut c_int,
    ) -> c_int;

    fn cntx_tool_read_limit() -> usize;
    fn cntx_tool_timeout_secs() -> u32;
    fn cntx_tool_timeout_max_secs() -> u32;
    fn cntx_glob_result_limit() -> u32;
    fn cntx_grep_line_limit() -> u32;
    fn cntx_tool_iteration_limit() -> u32;
    fn cntx_goal_default_max_steps() -> u32;

    fn cntx_goal_transition(status: c_int, event: c_int) -> c_int;
    fn cntx_goal_should_continue(status: c_int, steps_used: u32, max_steps: u32) -> c_int;
    fn cntx_goal_status_name(status: c_int) -> *const c_char;
    fn cntx_goal_parse_status(name: *const c_char, out_status: *mut c_int) -> c_int;

    fn cntx_context_default_budget() -> usize;
    fn cntx_context_split(user_turns: *const i32, count: usize) -> usize;
    fn cntx_optimize(
        raw: *const *const c_char,
        normalized: *const *const c_char,
        count: usize,
        out: *mut c_char,
        capacity: usize,
        written: *mut usize,
        duplicates: *mut usize,
    ) -> i32;
    fn cntx_estimate_tokens(characters: usize, words: usize) -> usize;
    fn cntx_context_score(
        content: *const c_char,
        terms: *const *const c_char,
        count: usize,
    ) -> usize;
    fn cntx_agent_next(
        status: i32,
        used: u32,
        limit: u32,
        interrupted: i32,
        denied: i32,
        stalled: u32,
    ) -> i32;
    fn cntx_model_rank(
        provider: i32,
        id: *const c_char,
        usage: *const c_char,
        size: *const c_char,
    ) -> i32;
    fn cntx_model_select(
        models: *const ModelCandidate,
        count: usize,
        target_rank: i32,
        default_id: *const c_char,
        reason: *mut i32,
    ) -> i64;
    fn cntx_context_should_compact(estimated_tokens: usize, budget: usize) -> c_int;
    fn cntx_context_budget_for_window(model_context_window: usize) -> usize;
    fn cntx_route_classify(estimated_tokens: usize, small: usize, medium: usize) -> c_int;
    fn cntx_counsel_classify(prompt: *const c_char) -> c_int;
    fn cntx_go_protocol_for_model(model_id: *const c_char) -> c_int;
}

/// Reject interior NUL bytes so C string inputs stay well-formed.
fn cstring(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| "string contains interior NUL bytes".to_string())
}

fn unsafe_cstr(ptr: *const c_char) -> Option<&'static str> {
    if ptr.is_null() {
        None
    } else {
        // # Safety
        // Static-storage C strings returned by the core.
        let slice = unsafe { CStr::from_ptr(ptr) };
        slice.to_str().ok()
    }
}

/// Permission decision from the C policy table.
pub fn permission_decide(mode: i32, operation: i32) -> i32 {
    unsafe { cntx_permission_decide(mode, operation) }
}

/// Canonical mode name.
pub fn mode_canonical_name(mode: i32) -> Option<&'static str> {
    unsafe { unsafe_cstr(cntx_mode_canonical_name(mode)) }
}

/// Human-readable mode description.
pub fn mode_description(mode: i32) -> Option<&'static str> {
    unsafe { unsafe_cstr(cntx_mode_description(mode)) }
}

/// Parse a mode name or legacy alias through the C core.
pub fn mode_parse(name: &str) -> Option<i32> {
    let c_name = cstring(name).ok()?;
    let mut out: c_int = -1;
    if unsafe { cntx_mode_parse(c_name.as_ptr(), &mut out) } == 0 && (0..=MODE_PLAN).contains(&out)
    {
        Some(out)
    } else {
        None
    }
}

/// Shift+Tab mode cycle through the C core.
pub fn mode_next(mode: i32) -> i32 {
    unsafe { cntx_mode_next(mode) }
}

/// Validate tool call arguments through the C core.
///
/// Non-string argument values are passed as null pointers so C can
/// distinguish a missing/non-string field from a present (possibly empty)
/// one; empty bodies are valid only for content/new_string, which C knows.
pub fn tool_validate(tool_name: &str, arguments: &serde_json::Value) -> Result<(), String> {
    let name = cstring(tool_name)?;
    let mut keys: Vec<CString> = Vec::new();
    let mut values: Vec<Option<CString>> = Vec::new();
    if let Some(map) = arguments.as_object() {
        for (key, value) in map {
            keys.push(cstring(key).map_err(|e| format!("{key}: {e}"))?);
            values.push(value.as_str().map(cstring).transpose()?);
        }
    }
    let key_ptrs: Vec<*const c_char> = keys.iter().map(|k| k.as_ptr()).collect();
    let value_ptrs: Vec<*const c_char> = values
        .iter()
        .map(|v| v.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()))
        .collect();
    let mut err = vec![0u8; 256];
    let rc = unsafe {
        cntx_tool_validate(
            name.as_ptr(),
            key_ptrs.as_ptr(),
            value_ptrs.as_ptr(),
            keys.len(),
            err.as_mut_ptr().cast(),
            err.len(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        let msg = CStr::from_bytes_until_nul(&err)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "invalid tool arguments".to_string());
        Err(msg)
    }
}

/// Bounded file read through the C core.
pub fn file_read(path: &Path, offset: u64, max_bytes: usize) -> Result<(String, bool), String> {
    let path_c = cstring(&path.to_string_lossy())?;
    let mut buf = vec![0u8; max_bytes.checked_add(1).ok_or("read size overflow")?];
    let mut written: usize = 0;
    let mut truncated: c_int = 0;
    let rc = unsafe {
        cntx_file_read(
            path_c.as_ptr(),
            offset,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut written,
            &mut truncated,
        )
    };
    match rc {
        0 => {
            // written <= buf.len() - 1 is guaranteed by the C core.
            let bytes = buf.get(..written).ok_or("invalid C read length")?;
            let text = String::from_utf8_lossy(bytes).into_owned();
            Ok((text, truncated != 0))
        }
        _ => Err(format!("read failed (code {rc})")),
    }
}

/// Bounded file write through the C core (host verifies containment first).
pub fn file_write(path: &Path, content: &str) -> Result<(), String> {
    let path_c = cstring(&path.to_string_lossy())?;
    let content_c = cstring(content)?;
    let rc = unsafe { cntx_file_write(path_c.as_ptr(), content_c.as_ptr(), content.len()) };
    match rc {
        0 => Ok(()),
        _ => Err(format!("write failed (code {rc})")),
    }
}

/// Single-match edit through the C core.
pub fn file_edit(path: &Path, old_text: &str, new_text: &str) -> Result<(), String> {
    let path_c = cstring(&path.to_string_lossy())?;
    let old_c = cstring(old_text)?;
    let new_c = cstring(new_text)?;
    let mut err = vec![0u8; 256];
    let rc = unsafe {
        cntx_file_edit(
            path_c.as_ptr(),
            old_c.as_ptr(),
            new_c.as_ptr(),
            err.as_mut_ptr().cast(),
            err.len(),
        )
    };
    match rc {
        0 => Ok(()),
        _ => {
            let msg = CStr::from_bytes_until_nul(&err)
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "edit failed".to_string());
            Err(if msg.is_empty() {
                format!("edit failed (code {rc})")
            } else {
                msg
            })
        }
    }
}

/// Result of a bounded command run.
pub struct CommandOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Bounded command execution through the C core. The cancel flag is a
/// host-owned `AtomicI32` polled by the C wait loop; set it non-zero to
/// terminate the child process group.
pub fn command_run(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &std::sync::atomic::AtomicI32,
) -> Result<CommandOutcome, String> {
    use std::os::raw::c_int;
    let command_c = cstring(command)?;
    let cwd_c = cstring(&cwd.to_string_lossy())?;
    let cap = tool_read_limit() + 256;
    let mut stdout_buf = vec![0u8; cap + 1];
    let mut stderr_buf = vec![0u8; cap + 1];
    let mut exit_code: c_int = -1;
    let mut timed_out: c_int = 0;
    let cancel_ptr = cancel.as_ptr() as *const c_int;
    let rc = unsafe {
        cntx_command_run(
            command_c.as_ptr(),
            cwd_c.as_ptr(),
            timeout.as_millis() as u64,
            cancel_ptr,
            stdout_buf.as_mut_ptr().cast(),
            stdout_buf.len(),
            stderr_buf.as_mut_ptr().cast(),
            stderr_buf.len(),
            &mut exit_code,
            &mut timed_out,
        )
    };
    if rc != 0 {
        return Err(format!("command run failed (code {rc})"));
    }
    let stdout = CStr::from_bytes_until_nul(&stdout_buf)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stderr = CStr::from_bytes_until_nul(&stderr_buf)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(CommandOutcome {
        stdout,
        stderr,
        exit_code,
        timed_out: timed_out != 0,
    })
}

/// Tool output read limit in bytes.
pub fn tool_read_limit() -> usize {
    unsafe { cntx_tool_read_limit() }
}

/// Default command timeout.
pub fn tool_timeout() -> Duration {
    Duration::from_secs(unsafe { cntx_tool_timeout_secs() } as u64)
}

/// Maximum allowed explicit command timeout.
pub fn tool_timeout_max() -> Duration {
    Duration::from_secs(unsafe { cntx_tool_timeout_max_secs() } as u64)
}

/// Maximum glob result paths.
pub fn glob_result_limit() -> u32 {
    unsafe { cntx_glob_result_limit() }
}

/// Maximum displayed grep lines.
pub fn grep_line_limit() -> u32 {
    unsafe { cntx_grep_line_limit() }
}

/// Tool iteration cap per ordinary prompt.
pub fn tool_iteration_limit() -> u32 {
    unsafe { cntx_tool_iteration_limit() }
}

/// Default goal step budget per batch.
pub fn goal_default_max_steps() -> u32 {
    unsafe { cntx_goal_default_max_steps() }
}

/// Goal state transition through the C state machine.
pub fn goal_transition(status: i32, event: i32) -> i32 {
    unsafe { cntx_goal_transition(status, event) }
}

/// Whether a goal loop should keep running.
pub fn goal_should_continue(status: i32, steps_used: u32, max_steps: u32) -> bool {
    unsafe { cntx_goal_should_continue(status, steps_used, max_steps) != 0 }
}

/// Goal status name.
pub fn goal_status_name(status: i32) -> Option<&'static str> {
    unsafe { unsafe_cstr(cntx_goal_status_name(status)) }
}

/// Parse a goal status name through the C core.
pub fn goal_parse_status(name: &str) -> Option<i32> {
    let c_name = cstring(name).ok()?;
    let mut out: c_int = -1;
    if unsafe { cntx_goal_parse_status(c_name.as_ptr(), &mut out) } == 0 && (0..=5).contains(&out) {
        Some(out)
    } else {
        None
    }
}

/// Default estimated input-token budget.
pub fn context_default_budget() -> usize {
    unsafe { cntx_context_default_budget() }
}

/// Whether the request should be compacted before sending.
pub fn context_should_compact(estimated_tokens: usize, budget: usize) -> bool {
    unsafe { cntx_context_should_compact(estimated_tokens, budget) != 0 }
}

/// Budget reduced for a known context window.
pub fn context_budget_for_window(window: usize) -> usize {
    unsafe { cntx_context_budget_for_window(window) }
}

/// Route-size classification (0 small, 1 medium, 2 large).
pub fn route_classify(
    estimated_tokens: usize,
    small_threshold: usize,
    medium_threshold: usize,
) -> i32 {
    unsafe { cntx_route_classify(estimated_tokens, small_threshold, medium_threshold) }
}

/// Counsel task classification (0 evaluate, 1 small-change, 2 refactor).
pub fn counsel_classify(prompt: &str) -> i32 {
    let Ok(prompt_c) = cstring(&prompt.chars().take(4096).collect::<String>()) else {
        return 0;
    };
    unsafe { cntx_counsel_classify(prompt_c.as_ptr()) }
}

/// OpenCode Go protocol for a model id (0 chat, 1 messages, 2 responses, 3 unknown).
pub fn go_protocol_for_model(model_id: &str) -> i32 {
    match cstring(model_id) {
        Ok(id) => unsafe { cntx_go_protocol_for_model(id.as_ptr()) },
        Err(_) => GO_PROTOCOL_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permission_table_matches_contract() {
        // auto-approve: reads and in-project writes allowed, shell asks
        assert_eq!(
            permission_decide(MODE_AUTO_APPROVE, OP_READ),
            DECISION_ALLOW
        );
        assert_eq!(
            permission_decide(MODE_AUTO_APPROVE, OP_WRITE),
            DECISION_ALLOW
        );
        assert_eq!(permission_decide(MODE_AUTO_APPROVE, OP_SHELL), DECISION_ASK);
        // all-approve allows everything
        assert_eq!(
            permission_decide(MODE_ALL_APPROVE, OP_SHELL),
            DECISION_ALLOW
        );
        // manual-approve asks for everything
        assert_eq!(
            permission_decide(MODE_MANUAL_APPROVE, OP_READ),
            DECISION_ASK
        );
        // file-only blocks shell/network
        assert_eq!(permission_decide(MODE_FILE_ONLY, OP_SHELL), DECISION_DENY);
        assert_eq!(permission_decide(MODE_FILE_ONLY, OP_WRITE), DECISION_ALLOW);
        // counsel matches auto
        assert_eq!(permission_decide(MODE_COUNSEL, OP_READ), DECISION_ALLOW);
        assert_eq!(permission_decide(MODE_COUNSEL, OP_WRITE), DECISION_ALLOW);
        // out-of-range denies
        assert_eq!(permission_decide(99, OP_READ), DECISION_DENY);
    }

    #[test]
    fn mode_names_and_aliases_parse() {
        assert_eq!(mode_parse("auto-approve"), Some(MODE_AUTO_APPROVE));
        assert_eq!(mode_parse("auto"), Some(MODE_AUTO_APPROVE));
        assert_eq!(mode_parse("all-approve"), Some(MODE_ALL_APPROVE));
        assert_eq!(mode_parse("allow"), Some(MODE_ALL_APPROVE));
        assert_eq!(mode_parse("manual-approve"), Some(MODE_MANUAL_APPROVE));
        assert_eq!(mode_parse("request-permission"), Some(MODE_MANUAL_APPROVE));
        assert_eq!(mode_parse("counsel"), Some(MODE_COUNSEL));
        assert_eq!(mode_parse("file-only"), Some(MODE_FILE_ONLY));
        assert_eq!(mode_parse("plan"), Some(MODE_PLAN));
        assert_eq!(mode_parse("bogus"), None);
        assert_eq!(mode_canonical_name(MODE_ALL_APPROVE), Some("all-approve"));
        assert_eq!(
            mode_canonical_name(MODE_MANUAL_APPROVE),
            Some("manual-approve")
        );
    }

    #[test]
    fn mode_cycle_is_three_canonical_modes() {
        assert_eq!(mode_next(MODE_AUTO_APPROVE), MODE_ALL_APPROVE);
        assert_eq!(mode_next(MODE_ALL_APPROVE), MODE_MANUAL_APPROVE);
        assert_eq!(mode_next(MODE_MANUAL_APPROVE), MODE_PLAN);
        assert_eq!(mode_next(MODE_PLAN), MODE_AUTO_APPROVE);
        // Legacy extras return to the canonical default.
        assert_eq!(mode_next(MODE_COUNSEL), MODE_AUTO_APPROVE);
        assert_eq!(mode_next(MODE_FILE_ONLY), MODE_AUTO_APPROVE);
    }

    #[test]
    fn tool_validation_rejects_bad_arguments() {
        assert!(tool_validate("write", &json!({"path": "x", "content": ""})).is_ok());
        assert!(tool_validate("write", &json!({"path": "", "content": "a"}))
            .unwrap_err()
            .contains("path cannot be empty"));
        assert!(tool_validate("write", &json!({"content": "a"}))
            .unwrap_err()
            .contains("missing required field: path"));
        assert!(tool_validate(
            "edit",
            &json!({"path": "x", "old_string": "", "new_string": "b"})
        )
        .unwrap_err()
        .contains("old_string cannot be empty"));
        assert!(tool_validate("rm", &json!({}))
            .unwrap_err()
            .contains("unknown tool"));
        assert!(tool_validate("bash", &json!({"command": "ls"})).is_ok());
    }

    #[test]
    fn file_tools_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("demo.txt");
        file_write(&path, "hello world\n").unwrap();
        let (content, truncated) = file_read(&path, 0, 100).unwrap();
        assert_eq!(content, "hello world\n");
        assert!(!truncated);

        file_edit(&path, "hello", "goodbye").unwrap();
        let (content, _) = file_read(&path, 0, 100).unwrap();
        assert_eq!(content, "goodbye world\n");

        // Ambiguous edit leaves the file untouched.
        file_write(&path, "a a\n").unwrap();
        assert!(file_edit(&path, "a", "b").is_err());
        let (content, _) = file_read(&path, 0, 100).unwrap();
        assert_eq!(content, "a a\n");
    }

    #[test]
    fn command_run_captures_output_and_timeout() {
        let temp = tempfile::tempdir().unwrap();
        let outcome = command_run(
            "echo out; echo err >&2; exit 3",
            temp.path(),
            tool_timeout(),
            &Default::default(),
        )
        .unwrap();
        assert!(outcome.stdout.contains("out"));
        assert!(outcome.stderr.contains("err"));
        assert_eq!(outcome.exit_code, 3);

        let cancelled = std::sync::atomic::AtomicI32::new(0);
        let outcome = command_run(
            "sleep 30",
            temp.path(),
            Duration::from_millis(300),
            &cancelled,
        )
        .unwrap();
        assert!(outcome.timed_out);
    }

    #[test]
    fn command_run_large_output_does_not_deadlock() {
        let temp = tempfile::tempdir().unwrap();
        let cancelled = std::sync::atomic::AtomicI32::new(0);
        // 2 MiB of output far exceeds pipe buffers.
        let outcome = command_run(
            "head -c 2097152 /dev/zero | tr '\\0' 'x'",
            temp.path(),
            tool_timeout(),
            &cancelled,
        )
        .unwrap();
        assert!(outcome.stdout.len() >= 24000);
    }

    #[test]
    fn goal_state_machine_rules() {
        assert_eq!(goal_transition(GOAL_NONE, GOAL_EVENT_START), GOAL_ACTIVE);
        assert_eq!(goal_transition(GOAL_ACTIVE, GOAL_EVENT_START), GOAL_ACTIVE);
        assert_eq!(goal_transition(GOAL_ACTIVE, GOAL_EVENT_PAUSE), GOAL_PAUSED);
        assert_eq!(goal_transition(GOAL_PAUSED, GOAL_EVENT_RESUME), GOAL_ACTIVE);
        assert_eq!(
            goal_transition(GOAL_ACTIVE, GOAL_EVENT_STEP_LIMIT),
            GOAL_PAUSED
        );
        assert_eq!(
            goal_transition(GOAL_ACTIVE, GOAL_EVENT_COMPLETE),
            GOAL_COMPLETED
        );
        assert_eq!(
            goal_transition(GOAL_ACTIVE, GOAL_EVENT_CANCEL),
            GOAL_CANCELLED
        );
        assert_eq!(
            goal_transition(GOAL_COMPLETED, GOAL_EVENT_START),
            GOAL_ACTIVE
        );
        assert!(goal_should_continue(GOAL_ACTIVE, 49, 50));
        assert!(!goal_should_continue(GOAL_ACTIVE, 50, 50));
        assert!(!goal_should_continue(GOAL_PAUSED, 0, 50));
        assert_eq!(goal_status_name(GOAL_BLOCKED), Some("blocked"));
        assert_eq!(goal_parse_status("completed"), Some(GOAL_COMPLETED));
    }

    #[test]
    fn context_budget_rules() {
        assert_eq!(context_default_budget(), 16000);
        assert!(context_should_compact(16001, 16000));
        assert!(!context_should_compact(16000, 16000));
        assert!(context_budget_for_window(8192) < 16000);
        assert_eq!(context_budget_for_window(0), 16000);
    }

    #[test]
    fn go_protocol_routing() {
        assert_eq!(go_protocol_for_model("glm-5.3-flash"), GO_PROTOCOL_CHAT);
        assert_eq!(go_protocol_for_model("kimi-k2"), GO_PROTOCOL_CHAT);
        assert_eq!(go_protocol_for_model("deepseek-v4"), GO_PROTOCOL_CHAT);
        assert_eq!(go_protocol_for_model("minimax-m2"), GO_PROTOCOL_MESSAGES);
        assert_eq!(go_protocol_for_model("qwen3-max"), GO_PROTOCOL_MESSAGES);
        assert_eq!(go_protocol_for_model("gpt-5.6-luna"), GO_PROTOCOL_RESPONSES);
        assert_eq!(go_protocol_for_model("grok-4.6"), GO_PROTOCOL_RESPONSES);
        assert_eq!(go_protocol_for_model("muse-spark"), GO_PROTOCOL_RESPONSES);
        assert_eq!(go_protocol_for_model("mystery-model"), GO_PROTOCOL_UNKNOWN);
    }
}

pub fn context_split(user_turns: &[i32]) -> usize {
    unsafe { cntx_context_split(user_turns.as_ptr(), user_turns.len()) }
}

pub fn optimize(prompt: &str) -> Result<(String, usize), String> {
    let raw: Vec<CString> = prompt
        .lines()
        .map(|s| cstring(s.trim_end()))
        .collect::<Result<_, _>>()?;
    let normalized: Vec<CString> = prompt
        .lines()
        .map(|s| cstring(&s.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect::<Result<_, _>>()?;
    let raw_ptrs: Vec<_> = raw.iter().map(|s| s.as_ptr()).collect();
    let norm_ptrs: Vec<_> = normalized.iter().map(|s| s.as_ptr()).collect();
    let mut out = vec![0u8; prompt.len().checked_add(1).ok_or("prompt too large")?];
    let (mut written, mut duplicates) = (0, 0);
    let rc = unsafe {
        cntx_optimize(
            raw_ptrs.as_ptr(),
            norm_ptrs.as_ptr(),
            raw.len(),
            out.as_mut_ptr().cast(),
            out.len(),
            &mut written,
            &mut duplicates,
        )
    };
    if rc != 0 {
        return Err(format!("prompt optimization failed ({rc})"));
    }
    let text = std::str::from_utf8(out.get(..written).ok_or("invalid output length")?)
        .map_err(|e| e.to_string())?;
    Ok((text.to_string(), duplicates))
}

pub fn estimate_tokens(characters: usize, words: usize) -> usize {
    unsafe { cntx_estimate_tokens(characters, words) }
}

pub fn context_score(content: &str, terms: &[String]) -> usize {
    let Ok(content) = cstring(content) else {
        return 0;
    };
    let terms: Vec<_> = terms.iter().filter_map(|s| cstring(s).ok()).collect();
    let ptrs: Vec<_> = terms.iter().map(|s| s.as_ptr()).collect();
    unsafe { cntx_context_score(content.as_ptr(), ptrs.as_ptr(), ptrs.len()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAction {
    Request,
    Stop,
    Pause,
}

pub fn agent_next(
    status: i32,
    used: u32,
    limit: u32,
    interrupted: bool,
    denied: bool,
    stalled: u32,
) -> AgentAction {
    match unsafe {
        cntx_agent_next(
            status,
            used,
            limit,
            interrupted.into(),
            denied.into(),
            stalled,
        )
    } {
        0 => AgentAction::Request,
        1 => AgentAction::Stop,
        _ => AgentAction::Pause,
    }
}

pub fn model_select(
    provider: &crate::config::ProviderKind,
    models: &[&crate::providers::ModelInfo],
    rank: i32,
    default: Option<&str>,
) -> Option<(String, &'static str)> {
    use crate::config::ProviderKind;
    let provider = match provider {
        ProviderKind::Anthropic => 0,
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => 1,
        ProviderKind::OllamaLocal => 2,
        ProviderKind::OllamaCloud => 3,
    };
    let ids: Vec<_> = models
        .iter()
        .map(|m| cstring(&m.id))
        .collect::<Result<_, _>>()
        .ok()?;
    let mut candidates = Vec::with_capacity(models.len());
    for (model, id) in models.iter().zip(&ids) {
        let lower = cstring(&model.id.to_lowercase()).ok()?;
        let usage = ["usage", "usage_level", "usageLevel"]
            .iter()
            .find_map(|key| model.metadata.get(*key).and_then(|v| v.as_str()))
            .unwrap_or("");
        let usage = cstring(&usage.to_lowercase()).ok()?;
        let size = model
            .metadata
            .get("details")
            .and_then(|v| v.get("parameter_size"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let size = cstring(&size.to_lowercase()).ok()?;
        candidates.push(ModelCandidate {
            id: id.as_ptr(),
            created: model.created_at.map_or(i64::MIN, |d| d.timestamp()),
            rank: unsafe {
                cntx_model_rank(provider, lower.as_ptr(), usage.as_ptr(), size.as_ptr())
            },
        });
    }
    let default_c = default.map(cstring).transpose().ok()?;
    let mut reason = 2;
    let index = unsafe {
        cntx_model_select(
            candidates.as_ptr(),
            candidates.len(),
            rank,
            default_c.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            &mut reason,
        )
    };
    let model = if index == -1 {
        default?.to_string()
    } else {
        models.get(usize::try_from(index).ok()?)?.id.clone()
    };
    Some((
        model,
        match reason {
            0 => "only available model",
            1 => "endpoint default model",
            _ => "prompt length after optimization",
        },
    ))
}
