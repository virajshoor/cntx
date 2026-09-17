/* C core self-test compiled and run by scripts/verify.sh with
 * AddressSanitizer and UndefinedBehaviorSanitizer. Covers success paths and
 * invalid-input paths for every C-owned decision. */

/* Feature-test macros must precede every header: mkdtemp is POSIX 2008, but
 * glibc under -std=c17 keeps parts of the family behind _DEFAULT_SOURCE and
 * macOS keeps it behind _DARWIN_C_SOURCE. */
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#define _DARWIN_C_SOURCE
#elif !defined(__APPLE__) && !defined(_DEFAULT_SOURCE)
#define _DEFAULT_SOURCE 1
#endif
#if !defined(__APPLE__) && !defined(_POSIX_C_SOURCE)
#define _POSIX_C_SOURCE 200809L
#endif

#include "cntx.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int failures = 0;

#define CHECK(condition, message)                                              \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "FAIL: %s\n", message);                            \
            failures++;                                                        \
        }                                                                      \
    } while (0)

static void test_permission_table(void) {
    CHECK(cntx_permission_decide(CNTX_MODE_AUTO_APPROVE, CNTX_OP_READ) ==
              CNTX_DECISION_ALLOW,
          "auto-approve allows reads");
    CHECK(cntx_permission_decide(CNTX_MODE_AUTO_APPROVE, CNTX_OP_WRITE) ==
              CNTX_DECISION_ASK,
          "auto-approve asks for writes");
    CHECK(cntx_permission_decide(CNTX_MODE_AUTO_APPROVE, CNTX_OP_SHELL) ==
              CNTX_DECISION_ASK,
          "auto-approve asks for shell");
    CHECK(cntx_permission_decide(CNTX_MODE_ALL_APPROVE, CNTX_OP_SHELL) ==
              CNTX_DECISION_ALLOW,
          "all-approve allows shell");
    CHECK(cntx_permission_decide(CNTX_MODE_MANUAL_APPROVE, CNTX_OP_READ) ==
              CNTX_DECISION_ASK,
          "manual-approve asks for reads");
    CHECK(cntx_permission_decide(CNTX_MODE_FILE_ONLY, CNTX_OP_SHELL) ==
              CNTX_DECISION_DENY,
          "file-only denies shell");
    CHECK(cntx_permission_decide(CNTX_MODE_FILE_ONLY, CNTX_OP_NETWORK) ==
              CNTX_DECISION_DENY,
          "file-only denies network");
    CHECK(cntx_permission_decide(CNTX_MODE_FILE_ONLY, CNTX_OP_WRITE) ==
              CNTX_DECISION_ALLOW,
          "file-only allows writes");
    CHECK(cntx_permission_decide(CNTX_MODE_COUNSEL, CNTX_OP_WRITE) ==
              CNTX_DECISION_ASK,
          "counsel asks for writes");
    /* Out-of-range inputs deny. */
    CHECK(cntx_permission_decide(99, CNTX_OP_READ) == CNTX_DECISION_DENY,
          "out-of-range mode denies");
    CHECK(cntx_permission_decide(-1, CNTX_OP_READ) == CNTX_DECISION_DENY,
          "negative mode denies");
    CHECK(cntx_permission_decide(CNTX_MODE_AUTO_APPROVE, 99) ==
              CNTX_DECISION_DENY,
          "out-of-range operation denies");
}

static void test_mode_helpers(void) {
    CHECK(strcmp(cntx_mode_canonical_name(CNTX_MODE_ALL_APPROVE),
                 "all-approve") == 0,
          "canonical name all-approve");
    CHECK(cntx_mode_canonical_name(42) == NULL, "invalid mode name is NULL");
    int mode = -1;
    CHECK(cntx_mode_parse("manual-approve", &mode) == CNTX_OK &&
              mode == CNTX_MODE_MANUAL_APPROVE,
          "parse manual-approve");
    CHECK(cntx_mode_parse("request-permission", &mode) == CNTX_OK &&
              mode == CNTX_MODE_MANUAL_APPROVE,
          "parse legacy alias");
    CHECK(cntx_mode_parse("bogus", &mode) == CNTX_ERR_NOT_FOUND,
          "parse unknown mode fails");
    CHECK(cntx_mode_parse(NULL, &mode) == CNTX_ERR_INVALID_ARGUMENT,
          "parse NULL fails");
    CHECK(cntx_mode_next(CNTX_MODE_AUTO_APPROVE) == CNTX_MODE_ALL_APPROVE,
          "cycle auto to all");
    CHECK(cntx_mode_next(CNTX_MODE_MANUAL_APPROVE) ==
              CNTX_MODE_AUTO_APPROVE,
          "cycle manual to auto");
    CHECK(cntx_mode_next(CNTX_MODE_FILE_ONLY) == CNTX_MODE_AUTO_APPROVE,
          "legacy mode returns to auto");
}

static void test_tool_validation(void) {
    char err[128];
    CHECK(cntx_tool_validate("write", (const char *[]){"path", "content"},
                             (const char *[]){"x", ""}, 2, err, sizeof(err)) ==
              CNTX_OK,
          "empty content allowed");
    CHECK(cntx_tool_validate("write", (const char *[]){"path", "content"},
                             (const char *[]){"", "a"}, 2, err, sizeof(err)) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "empty path rejected");
    CHECK(cntx_tool_validate("edit", (const char *[]){"path", "old_string"},
                             (const char *[]){"f", ""}, 2, err,
                             sizeof(err)) == CNTX_ERR_INVALID_ARGUMENT,
          "missing new_string rejected");
    CHECK(cntx_tool_validate("rm", NULL, NULL, 0, err, sizeof(err)) ==
              CNTX_ERR_NOT_FOUND,
          "unknown tool rejected");
    CHECK(cntx_tool_validate("read", (const char *[]){"path"},
                             (const char *[]){NULL}, 1, err, sizeof(err)) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "null value rejected as missing");
}

static void test_file_tools(const char *dir) {
    char path[4096];
    snprintf(path, sizeof(path), "%s/cfile.txt", dir);
    CHECK(cntx_file_write(path, "hello world\n", 12) == CNTX_OK,
          "write creates file");
    char buf[256];
    char err[256];
    size_t written = 0;
    int truncated = 0;
    CHECK(cntx_file_read(path, 0, buf, sizeof(buf), &written, &truncated) ==
              CNTX_OK && written == 12 && truncated == 0 &&
              strcmp(buf, "hello world\n") == 0,
          "read matches write");
    CHECK(cntx_file_edit(path, "hello", "goodbye", err, sizeof(err)) ==
              CNTX_OK,
          "single-match edit succeeds");
    CHECK(cntx_file_write(path, "dup and dup again", 16) == CNTX_OK,
          "rewrite for ambiguity test");
    CHECK(cntx_file_edit(path, "dup", "gone", err, sizeof(err)) ==
              CNTX_ERR_AMBIGUOUS,
          "two-match edit rejected");
    char missing[4096];
    snprintf(missing, sizeof(missing), "%s/does-not-exist.txt", dir);
    CHECK(cntx_file_edit(missing, "a", "b", err, sizeof(err)) ==
              CNTX_ERR_NOT_FOUND,
          "edit missing file fails");
    CHECK(cntx_file_edit(path, "", "x", err, sizeof(err)) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "empty old_string rejected");
    CHECK(cntx_file_edit(path, "", "x", NULL, 0) == CNTX_ERR_INVALID_ARGUMENT,
          "null err buffer rejected");
    /* Truncated read reports the truncation flag. */
    CHECK(cntx_file_read(path, 0, buf, 4, &written, &truncated) == CNTX_OK &&
              truncated == 1 && written == 3,
          "bounded read truncates");
    CHECK(cntx_file_read(path, UINT64_MAX, buf, sizeof(buf), &written, &truncated) == CNTX_ERR_INVALID_ARGUMENT, "read overflow rejected");
    CHECK(cntx_file_read(path, 10000, buf, sizeof(buf), &written, &truncated) == CNTX_OK && written == 0, "read beyond EOF empty");
    CHECK(cntx_file_write(path, "a\0b", 3) == CNTX_OK, "binary fixture");
    CHECK(cntx_file_edit(path, "b", "c", err, sizeof(err)) == CNTX_OK, "edit past NUL does not crash");
    /* Invalid inputs. */
    CHECK(cntx_file_read(NULL, 0, buf, sizeof(buf), &written, &truncated) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "read NULL path rejected");
    CHECK(cntx_file_write(path, NULL, 1) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "write null content fails");
}

static void test_command_run(const char *dir) {
    char out[512], err[512];
    int exit_code = 0, timed_out = 0;
    const volatile int cancel = 0;
    CHECK(cntx_command_run("printf hello; printf oops >&2; exit 3", dir, 5000,
                           &cancel, out, sizeof(out), err, sizeof(err),
                           &exit_code, &timed_out) == CNTX_OK,
          "command run succeeds");
    CHECK(exit_code == 3 && timed_out == 0, "exit code captured");
    CHECK(strstr(out, "hello") != NULL, "stdout captured");
    CHECK(strstr(err, "oops") != NULL, "stderr captured");

    CHECK(cntx_command_run("sleep 5", dir, 200, &cancel, out, sizeof(out),
                           err, sizeof(err), &exit_code, &timed_out) ==
              CNTX_OK,
          "timeout run returns");
    CHECK(timed_out == 1 && exit_code == -1, "timeout reported");

    CHECK(cntx_command_run("", dir, 1000, &cancel, out, sizeof(out), err,
                           sizeof(err), &exit_code, &timed_out) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "empty command rejected");
    CHECK(cntx_command_run("true", dir, 1000, &cancel, NULL, 10, err,
                           sizeof(err), &exit_code, &timed_out) ==
              CNTX_ERR_INVALID_ARGUMENT,
          "null stdout buffer rejected");

    /* Host cancellation terminates the child. */
    volatile int cancel_now = 1;
    CHECK(cntx_command_run("sleep 5", dir, 30000, &cancel_now, out,
                           sizeof(out), err, sizeof(err), &exit_code,
                           &timed_out) == CNTX_OK,
          "cancelled run returns");
    CHECK(exit_code == -1, "cancelled child terminated");
}

static void test_goal_machine(void) {
    CHECK(cntx_goal_transition(CNTX_GOAL_NONE, CNTX_GOAL_EVENT_START) ==
              CNTX_GOAL_ACTIVE,
          "start activates");
    CHECK(cntx_goal_transition(CNTX_GOAL_ACTIVE, CNTX_GOAL_EVENT_START) ==
              CNTX_GOAL_ACTIVE,
          "start on active keeps active");
    CHECK(cntx_goal_transition(CNTX_GOAL_ACTIVE,
                               CNTX_GOAL_EVENT_STEP_LIMIT) == CNTX_GOAL_PAUSED,
          "step limit pauses");
    CHECK(cntx_goal_transition(CNTX_GOAL_ACTIVE,
                               CNTX_GOAL_EVENT_COMPLETE) ==
              CNTX_GOAL_COMPLETED,
          "complete completes");
    CHECK(cntx_goal_transition(CNTX_GOAL_PAUSED, CNTX_GOAL_EVENT_RESUME) ==
              CNTX_GOAL_ACTIVE,
          "resume resumes");
    CHECK(cntx_goal_should_continue(CNTX_GOAL_ACTIVE, 49, 50) == 1,
          "continue under cap");
    CHECK(cntx_goal_should_continue(CNTX_GOAL_ACTIVE, 50, 50) == 0,
          "stop at cap");
    CHECK(cntx_goal_should_continue(CNTX_GOAL_PAUSED, 0, 50) == 0,
          "paused stops");
    CHECK(strcmp(cntx_goal_status_name(CNTX_GOAL_BLOCKED), "blocked") == 0,
          "status name");
    int status = -1;
    CHECK(cntx_goal_parse_status("completed", &status) == CNTX_OK &&
              status == CNTX_GOAL_COMPLETED,
          "parse status");
    CHECK(cntx_goal_parse_status("nope", &status) == CNTX_ERR_NOT_FOUND,
          "parse bad status");
}

static void test_context_and_routing(void) {
    CHECK(cntx_context_default_budget() == 16000, "default budget");
    CHECK(cntx_context_should_compact(16001, 16000) == 1, "compact over");
    CHECK(cntx_context_should_compact(16000, 16000) == 0, "no compact under");
    CHECK(cntx_context_budget_for_window(0) == 16000, "unknown window");
    CHECK(cntx_context_budget_for_window(8192) < 16000, "small window reduced");

    CHECK(cntx_route_classify(5, 10, 100) == CNTX_ROUTE_SMALL, "route small");
    CHECK(cntx_route_classify(50, 10, 100) == CNTX_ROUTE_MEDIUM, "route medium");
    CHECK(cntx_route_classify(500, 10, 100) == CNTX_ROUTE_LARGE, "route large");

    CHECK(cntx_counsel_classify("fix the typo") == CNTX_TASK_SMALL_CHANGE,
          "counsel small");
    CHECK(cntx_counsel_classify("REFACTOR the module") == CNTX_TASK_REFACTOR,
          "counsel refactor case-insensitive");
    CHECK(cntx_counsel_classify("review this") == CNTX_TASK_EVALUATE,
          "counsel evaluate");
    char large[5000]; memset(large, 'x', sizeof(large) - 1); large[sizeof(large) - 1] = 0;
    CHECK(cntx_counsel_classify(large) == CNTX_TASK_EVALUATE, "counsel bounded prefix");
    CHECK(cntx_counsel_classify(NULL) == CNTX_TASK_EVALUATE,
          "counsel NULL safe");

    CHECK(cntx_go_protocol_for_model("glm-5.3-flash") ==
              CNTX_GO_PROTOCOL_CHAT,
          "go glm chat");
    CHECK(cntx_go_protocol_for_model("kimi-k2") == CNTX_GO_PROTOCOL_CHAT,
          "go kimi chat");
    CHECK(cntx_go_protocol_for_model("minimax-m3") ==
              CNTX_GO_PROTOCOL_MESSAGES,
          "go minimax messages");
    CHECK(cntx_go_protocol_for_model("qwen3-max") ==
              CNTX_GO_PROTOCOL_MESSAGES,
          "go qwen messages");
    CHECK(cntx_go_protocol_for_model("gpt-5.6-luna") ==
              CNTX_GO_PROTOCOL_RESPONSES,
          "go gpt responses");
    CHECK(cntx_go_protocol_for_model("grok-4.6") ==
              CNTX_GO_PROTOCOL_RESPONSES,
          "go grok responses");
    CHECK(cntx_go_protocol_for_model("muse-spark") ==
              CNTX_GO_PROTOCOL_RESPONSES,
          "go muse responses");
    CHECK(cntx_go_protocol_for_model("unknown-model") ==
              CNTX_GO_PROTOCOL_UNKNOWN,
          "go unknown family");
}

static void test_limits(void) {
    CHECK(cntx_tool_read_limit() == 24000, "read limit");
    CHECK(cntx_tool_command_cap_bytes() == 10 * 1024 * 1024, "temp cap");
    CHECK(cntx_tool_timeout_secs() == 60, "default timeout");
    CHECK(cntx_tool_timeout_max_secs() == 600, "max timeout");
    CHECK(cntx_glob_result_limit() == 500, "glob limit");
    CHECK(cntx_grep_line_limit() == 50, "grep limit");
    CHECK(cntx_tool_iteration_limit() == 25, "iteration limit");
    CHECK(cntx_goal_default_max_steps() == 50, "goal steps");
}

int main(void) {
    char dir_template[] = "/tmp/cntx-selftest-XXXXXX";
    char *dir = mkdtemp(dir_template);
    if (dir == NULL) {
        fprintf(stderr, "FAIL: mkdtemp\n");
        return 1;
    }
    test_permission_table();
    test_mode_helpers();
    test_tool_validation();
    test_file_tools(dir);
    test_command_run(dir);
    test_goal_machine();
    test_context_and_routing();
    test_limits();
    char file[4096];
    snprintf(file, sizeof(file), "%s/cfile.txt", dir);
    unlink(file);
    rmdir(dir);
    if (failures > 0) {
        fprintf(stderr, "%d C self-test failure(s)\n", failures);
        return 1;
    }
    printf("C self-test: all checks passed\n");
    return 0;
}
