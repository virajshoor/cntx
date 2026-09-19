/*
 * Cntx C core ABI.
 *
 * The C core owns product decisions that must have exactly one
 * implementation: permission/approval semantics, tool argument validation,
 * file tool behavior, bounded command execution, agent/goal state
 * transitions, context budget rules, routing classification, and OpenCode Go
 * protocol selection.
 *
 * Ownership and lifetime rules:
 * - The Rust host passes parsed strings/buffers in; C never retains any
 *   pointer to host memory after a call returns.
 * - Every buffer the host provides carries its length explicitly. C never
 *   writes past the length and always NUL-terminates text buffers.
 * - C returns codes, not allocated memory, except static string literals
 *   (mode names, goal status names) which have static storage duration.
 * - The host owns filesystem containment checks. Containment is verified in
 *   Rust (canonical paths) before any write/edit entry point is called; the
 *   C layer performs the operation against the already-validated path.
 *
 * All enum parameters are passed as int32 values at the ABI boundary; the
 * Rust wrapper converts between Rust enums and these integer codes.
 */

#ifndef CNTX_H
#define CNTX_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Return codes shared by the C core. Negative values are errors. */
#define CNTX_OK 0
#define CNTX_ERR_INVALID_ARGUMENT (-1)
#define CNTX_ERR_NOT_FOUND (-2)
#define CNTX_ERR_IO (-3)
#define CNTX_ERR_TIMEOUT (-4)
#define CNTX_ERR_INTERRUPTED (-5)
#define CNTX_ERR_CAP_EXCEEDED (-6)
#define CNTX_ERR_AMBIGUOUS (-7)

/* ---- Approval modes and permission decisions ---- */

typedef enum cntx_decision {
    CNTX_DECISION_ALLOW = 0,
    CNTX_DECISION_ASK = 1,
    CNTX_DECISION_DENY = 2
} cntx_decision_t;

typedef enum cntx_mode {
    CNTX_MODE_AUTO_APPROVE = 0, /* reads and in-project writes allowed; commands ask */
    CNTX_MODE_COUNSEL = 1,      /* counsel evaluation; same policy as auto */
    CNTX_MODE_ALL_APPROVE = 2,  /* permitted tools run without prompting */
    CNTX_MODE_MANUAL_APPROVE = 3, /* every tool asks first */
    CNTX_MODE_FILE_ONLY = 4,    /* files ok; shell/network denied */
    CNTX_MODE_PLAN = 5          /* read/glob/grep only; deny write/edit/bash */
} cntx_mode_t;

typedef enum cntx_operation {
    CNTX_OP_READ = 0,
    CNTX_OP_WRITE = 1,
    CNTX_OP_SHELL = 2,
    CNTX_OP_NETWORK = 3
} cntx_op_t;

/* The single implementation of mode semantics used by every caller. */
cntx_decision_t cntx_permission_decide(int mode, int operation);

/* Canonical mode name (static storage; NULL when mode is out of range). */
const char *cntx_mode_canonical_name(int mode);

/* Description shown in /mode output (static storage). */
const char *cntx_mode_description(int mode);

/* Parse a canonical or legacy alias name. CNTX_OK or CNTX_ERR_NOT_FOUND. */
int cntx_mode_parse(const char *name, int *out_mode);

/* Shift+Tab cycle: auto-approve -> all-approve -> manual-approve ->
 * auto-approve. Legacy extra modes (counsel, file-only) return to
 * auto-approve. */
int cntx_mode_next(int mode);

/* ---- Tool validation and file/command execution ---- */

/* Validate a tool call. `keys`/`values` are parallel arrays with `count`
 * entries (values may be NULL when the field is missing). Writes a short
 * error message into err/err_len when invalid. Required fields may not be
 * missing or empty; write/edit body fields (content, new_string) may be
 * empty when explicitly provided. */
int cntx_tool_validate(const char *tool_name, const char *const *keys,
                       const char *const *values, size_t count, char *err,
                       size_t err_len);

/* Bounded file read. Reads at most buf_len-1 bytes starting at byte offset
 * into buf and NUL-terminates it. `truncated` is set to 1 when the file
 * extends past the window. */
int cntx_file_read(const char *path, uint64_t offset, char *buf,
                   size_t buf_len, size_t *written, int *truncated);

/* Bounded file write against a host-validated path. Creates missing parent
 * directories. Existing file permissions are preserved. */
int cntx_file_write(const char *path, const char *content, size_t content_len);

/* Edit with exactly-one-match semantics. Returns CNTX_ERR_AMBIGUOUS (file
 * untouched) for zero or multiple matches and CNTX_ERR_INVALID_ARGUMENT for
 * an empty old_string. On success the file is rewritten in place without
 * changing its permissions. */
int cntx_file_edit(const char *path, const char *old_text,
                   const char *new_text, char *err, size_t err_len);

/* Bounded command execution.
 *
 * Runs `command` with /bin/sh -c in `cwd` in its own process group, streams
 * output to temporary files, polls the deadline and the host-owned cancel
 * flag, and terminates/reaps the process group on timeout, cancellation, or
 * the per-stream temporary-file cap. stdout_buf/stderr_buf receive up to
 * buf_len-1 bytes, NUL-terminated; when the stream was larger a truncation
 * marker is appended. exit_code is the child status or -1 when killed.
 * timed_out is 1 on timeout, 0 otherwise. */
int cntx_command_run(const char *command, const char *cwd,
                     uint64_t timeout_ms, const volatile int *cancel,
                     char *stdout_buf, size_t stdout_len, char *stderr_buf,
                     size_t stderr_len, int *exit_code, int *timed_out);

/* ---- Limits (single source of truth for every caller) ---- */

size_t cntx_tool_read_limit(void);          /* read result bytes */
size_t cntx_tool_command_cap_bytes(void);   /* temp file cap per stream */
uint32_t cntx_tool_timeout_secs(void);      /* default command timeout */
uint32_t cntx_tool_timeout_max_secs(void);  /* optional timeout upper bound */
uint32_t cntx_glob_result_limit(void);      /* max glob paths */
uint32_t cntx_grep_line_limit(void);        /* max displayed grep lines */
uint32_t cntx_tool_iteration_limit(void);   /* tool steps per ordinary prompt */
uint32_t cntx_goal_default_max_steps(void); /* goal steps per batch */

/* ---- Agent / goal state machine ---- */

typedef enum cntx_goal_status {
    CNTX_GOAL_NONE = 0,
    CNTX_GOAL_ACTIVE = 1,
    CNTX_GOAL_PAUSED = 2,
    CNTX_GOAL_BLOCKED = 3,
    CNTX_GOAL_COMPLETED = 4,
    CNTX_GOAL_CANCELLED = 5
} cntx_goal_status_t;

typedef enum cntx_goal_event {
    CNTX_GOAL_EVENT_START = 0,
    CNTX_GOAL_EVENT_PAUSE = 1,
    CNTX_GOAL_EVENT_RESUME = 2,
    CNTX_GOAL_EVENT_CANCEL = 3,
    CNTX_GOAL_EVENT_COMPLETE = 4,
    CNTX_GOAL_EVENT_BLOCK = 5,
    CNTX_GOAL_EVENT_STEP_LIMIT = 6,
    CNTX_GOAL_EVENT_PROVIDER_FAILURE = 7
} cntx_goal_event_t;

/* Invalid transitions leave the status unchanged so callers can detect and
 * report them instead of silently corrupting state. */
cntx_goal_status_t cntx_goal_transition(int status, int event);

/* 1 = keep the goal loop running, 0 = stop (paused/blocked/done or the
 * step cap is exhausted). */
int cntx_goal_should_continue(int status, uint32_t steps_used,
                              uint32_t max_steps);

const char *cntx_goal_status_name(int status);
int cntx_goal_parse_status(const char *name, int *out_status);

/* ---- Context budget rules ---- */

/* Configurable default estimated input-token budget (tokens). */
size_t cntx_context_default_budget(void);
size_t cntx_context_split(const int32_t *user_turns, size_t count);

/* Host supplies Unicode-normalized lines; C owns fence/blank/dedup policy.
 * Strings are borrowed, NUL-terminated. Output is caller-owned. */
int32_t cntx_optimize(const char *const *raw, const char *const *normalized,
                     size_t count, char *out, size_t capacity,
                     size_t *written, size_t *duplicates);
size_t cntx_estimate_tokens(size_t characters, size_t words);
size_t cntx_context_score(const char *content, const char *const *terms, size_t count);

/* Agent returns a typed action to its host; no callbacks or retained pointers. */
#define CNTX_AGENT_REQUEST 0
#define CNTX_AGENT_STOP 1
#define CNTX_AGENT_PAUSE 2
int32_t cntx_agent_next(int32_t status, uint32_t used, uint32_t limit,
                        int32_t interrupted, int32_t denied, uint32_t stalled);

/* Rank codes: small=0, medium=1, large=2, unknown=3. Provider codes:
 * Anthropic=0, OpenAI-compatible=1, Ollama local=2, Ollama cloud=3.
 * Host extracts metadata and lowercases Unicode strings before calling. */
int32_t cntx_model_rank(int32_t provider, const char *id, const char *usage,
                       const char *parameter_size);
typedef struct cntx_model_candidate {
    const char *id;
    int64_t created;
    int32_t rank;
} cntx_model_candidate;
/* Return candidate index, -1 for configured default absent from empty catalog,
 * -2 for no candidate/invalid input. reason: 0=only model, 1=default, 2=tier. */
int64_t cntx_model_select(const cntx_model_candidate *models, size_t count,
                          int32_t target_rank, const char *default_id, int32_t *reason);

/* 1 when a request estimated at `estimated_tokens` should be compacted
 * before sending. */
int cntx_context_should_compact(size_t estimated_tokens, size_t budget);

/* Budget reduced for a known model context window (reserve output space).
 * Pass 0 when the window is unknown to get the default budget. */
size_t cntx_context_budget_for_window(size_t model_context_window);

/* ---- Routing classification ---- */

typedef enum cntx_route_size {
    CNTX_ROUTE_SMALL = 0,
    CNTX_ROUTE_MEDIUM = 1,
    CNTX_ROUTE_LARGE = 2
} cntx_route_size_t;

cntx_route_size_t cntx_route_classify(size_t estimated_tokens,
                                      size_t small_threshold,
                                      size_t medium_threshold);

/* Counsel classification: evaluate / small-change / refactor. */
typedef enum cntx_task {
    CNTX_TASK_EVALUATE = 0,
    CNTX_TASK_SMALL_CHANGE = 1,
    CNTX_TASK_REFACTOR = 2
} cntx_task_t;

cntx_task_t cntx_counsel_classify(const char *prompt);

/* ---- OpenCode Go protocol selection ---- */

typedef enum cntx_go_protocol {
    CNTX_GO_PROTOCOL_CHAT = 0,      /* /chat/completions */
    CNTX_GO_PROTOCOL_MESSAGES = 1,  /* /messages (Anthropic-compatible) */
    CNTX_GO_PROTOCOL_RESPONSES = 2, /* /responses (OpenAI Responses) */
    CNTX_GO_PROTOCOL_UNKNOWN = 3
} cntx_go_protocol_t;

/* Route a Go model id to its API protocol by model family. Unknown
 * families return CNTX_GO_PROTOCOL_UNKNOWN; callers must request an
 * explicit endpoint protocol override instead of guessing. */
cntx_go_protocol_t cntx_go_protocol_for_model(const char *model_id);

#ifdef __cplusplus
}
#endif

#endif /* CNTX_H */
