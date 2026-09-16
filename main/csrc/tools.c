/* Tool validation, file tool behavior, and bounded command execution.
 *
 * File tools operate against paths that the Rust host has already resolved
 * and verified for containment (canonical paths, symlink/traversal checks).
 * The C layer performs the actual bounded file operations and subprocess
 * management, including output caps, timeout polling, cancellation, and
 * process-group termination. */
#include "cntx.h"

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

/* ---- Limits ---- */

size_t cntx_tool_read_limit(void) { return 24000; }
size_t cntx_tool_command_cap_bytes(void) { return 10 * 1024 * 1024; }
uint32_t cntx_tool_timeout_secs(void) { return 60; }
uint32_t cntx_tool_timeout_max_secs(void) { return 600; }
uint32_t cntx_glob_result_limit(void) { return 500; }
uint32_t cntx_grep_line_limit(void) { return 50; }
uint32_t cntx_tool_iteration_limit(void) { return 25; }
uint32_t cntx_goal_default_max_steps(void) { return 50; }

/* ---- Tool argument validation ---- */

/* Fields whose value is allowed to be an empty string when present. */
static int is_body_field(const char *key) {
    return strcmp(key, "content") == 0 || strcmp(key, "new_string") == 0;
}

int cntx_tool_validate(const char *tool_name, const char *const *keys,
                       const char *const *values, size_t count, char *err,
                       size_t err_len) {
    if (tool_name == NULL || err == NULL || err_len == 0 || (keys == NULL) != (values == NULL) ||
        (count > 0 && (keys == NULL || values == NULL))) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }

    /* Required fields per tool. */
    const char *required[4];
    size_t required_count = 0;
    int known = 1;
    if (strcmp(tool_name, "read") == 0) {
        required[required_count++] = "path";
    } else if (strcmp(tool_name, "write") == 0) {
        required[required_count++] = "path";
        required[required_count++] = "content";
    } else if (strcmp(tool_name, "edit") == 0) {
        required[required_count++] = "path";
        required[required_count++] = "old_string";
        required[required_count++] = "new_string";
    } else if (strcmp(tool_name, "bash") == 0) {
        required[required_count++] = "command";
    } else if (strcmp(tool_name, "glob") == 0 ||
               strcmp(tool_name, "grep") == 0) {
        required[required_count++] = "pattern";
    } else {
        known = 0;
    }
    if (!known) {
        snprintf(err, err_len, "unknown tool: %s", tool_name);
        return CNTX_ERR_NOT_FOUND;
    }

    /* All provided values must be non-NULL (the host maps missing JSON
     * fields to NULL) and non-empty unless the field is a write body. */
    for (size_t i = 0; i < count; i++) {
        if (keys[i] == NULL) return CNTX_ERR_INVALID_ARGUMENT;
        if (values[i] == NULL) {
            continue; /* missing field; caught by the required check below */
        }
        if (values[i][0] == '\0' && !is_body_field(keys[i])) {
            snprintf(err, err_len, "%s cannot be empty", keys[i]);
            return CNTX_ERR_INVALID_ARGUMENT;
        }
    }
    for (size_t r = 0; r < required_count; r++) {
        size_t found = 0;
        for (size_t i = 0; i < count; i++) {
            if (strcmp(keys[i], required[r]) == 0 && values[i] != NULL) {
                found = 1;
                break;
            }
        }
        if (!found) {
            snprintf(err, err_len, "missing required field: %s", required[r]);
            return CNTX_ERR_INVALID_ARGUMENT;
        }
    }
    err[0] = '\0';
    return CNTX_OK;
}

/* ---- Bounded file read ---- */

int cntx_file_read(const char *path, uint64_t offset, char *buf,
                   size_t buf_len, size_t *written, int *truncated) {
    if (path == NULL || buf == NULL || buf_len == 0) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        return CNTX_ERR_NOT_FOUND;
    }
    off_t start = (off_t)offset;
    if (start < 0 || (uint64_t)start != offset) {
        close(fd);
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    off_t size = lseek(fd, 0, SEEK_END);
    if (size < 0 || lseek(fd, start, SEEK_SET) < 0) {
        close(fd);
        return CNTX_ERR_IO;
    }
    uint64_t remaining = size > start ? (uint64_t)(size - start) : 0;
    int out_truncated = remaining > buf_len - 1;
    size_t to_read = buf_len - 1;
    if (remaining < to_read) {
        to_read = (size_t)remaining;
    }
    size_t total = 0;
    while (total < to_read) {
        ssize_t n = read(fd, buf + total, to_read - total);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            close(fd);
            return CNTX_ERR_IO;
        }
        if (n == 0) {
            break;
        }
        total += (size_t)n;
    }
    close(fd);
    buf[total] = '\0';
    if (written != NULL) {
        *written = total;
    }
    if (truncated != NULL) {
        *truncated = out_truncated;
    }
    return CNTX_OK;
}

/* Create missing parent directories, like mkdir -p. */
static int mkdir_p(const char *path) {
    char buf[4096];
    size_t len = strlen(path);
    if (len == 0 || len >= sizeof(buf)) {
        return -1;
    }
    memcpy(buf, path, len + 1);
    for (char *p = buf + 1; *p != '\0'; p++) {
        if (*p == '/') {
            *p = '\0';
            if (mkdir(buf, 0777) != 0 && errno != EEXIST) {
                return -1;
            }
            *p = '/';
        }
    }
    if (mkdir(buf, 0777) != 0 && errno != EEXIST) {
        return -1;
    }
    return 0;
}

/* Replace only after a complete write. Preserve mode; failed writes leave
 * the original intact. Refuse final symlinks (host resolves safe targets). */
static int write_truncate(const char *path, const char *content,
                          size_t content_len) {
    struct stat original;
    int exists = lstat(path, &original) == 0;
    if ((exists && !S_ISREG(original.st_mode)) || (!exists && errno != ENOENT))
        return CNTX_ERR_IO;
    char temp[4096];
    int len = snprintf(temp, sizeof(temp), "%s.cntx-XXXXXX", path);
    if (len < 0 || (size_t)len >= sizeof(temp)) return CNTX_ERR_INVALID_ARGUMENT;
    int fd = mkstemp(temp);
    if (fd < 0) {
        return CNTX_ERR_IO;
    }
    size_t total = 0;
    int rc = CNTX_OK;
    while (total < content_len) {
        ssize_t n = write(fd, content + total, content_len - total);
        if (n <= 0) {
            if (errno == EINTR) {
                continue;
            }
            rc = CNTX_ERR_IO;
            break;
        }
        total += (size_t)n;
    }
    if (rc == CNTX_OK && exists && fchmod(fd, original.st_mode & 07777) != 0)
        rc = CNTX_ERR_IO;
    if (rc == CNTX_OK && fsync(fd) != 0) rc = CNTX_ERR_IO;
    if (close(fd) != 0) rc = CNTX_ERR_IO;
    if (rc == CNTX_OK && rename(temp, path) != 0) rc = CNTX_ERR_IO;
    if (rc != CNTX_OK) unlink(temp);
    return rc;
}

int cntx_file_write(const char *path, const char *content, size_t content_len) {
    if (path == NULL || content == NULL) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    /* Create the parent directory chain before the write. */
    char parent[4096];
    const char *slash = strrchr(path, '/');
    if (slash != NULL && slash != path) {
        size_t plen = (size_t)(slash - path);
        if (plen >= sizeof(parent)) {
            return CNTX_ERR_INVALID_ARGUMENT;
        }
        memcpy(parent, path, plen);
        parent[plen] = '\0';
        if (mkdir_p(parent) != 0) {
            return CNTX_ERR_IO;
        }
    }
    return write_truncate(path, content, content_len);
}

int cntx_file_edit(const char *path, const char *old_text,
                   const char *new_text, char *err, size_t err_len) {
    if (path == NULL || old_text == NULL || new_text == NULL || err == NULL ||
        err_len == 0) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    err[0] = '\0';
    size_t old_len = strlen(old_text);
    if (old_len == 0) {
        snprintf(err, err_len, "old_string cannot be empty");
        return CNTX_ERR_INVALID_ARGUMENT;
    }

    /* Read the whole file; cap to a sane bound to avoid runaway memory. */
    size_t cap = 16 * 1024 * 1024;
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        snprintf(err, err_len, "cannot open %s", path);
        return CNTX_ERR_NOT_FOUND;
    }
    char *content = malloc(cap + 1);
    if (content == NULL) {
        close(fd);
        return CNTX_ERR_IO;
    }
    size_t clen = 0;
    for (;;) {
        if (clen >= cap) {
            free(content);
            close(fd);
            snprintf(err, err_len, "file exceeds the 16 MiB edit limit");
            return CNTX_ERR_CAP_EXCEEDED;
        }
        ssize_t n = read(fd, content + clen, cap - clen);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            free(content);
            close(fd);
            return CNTX_ERR_IO;
        }
        if (n == 0) {
            break;
        }
        clen += (size_t)n;
    }
    close(fd);
    content[clen] = '\0';

    /* Count exact matches (overlapping matches of the same text cannot
     * occur more than once per position; plain sliding count is right). */
    size_t matches = 0, before = 0;
    if (clen >= old_len) {
        for (size_t i = 0; i + old_len <= clen; i++) {
            if (memcmp(content + i, old_text, old_len) == 0) {
                matches++;
                before = i;
            }
        }
    }
    if (matches != 1) {
        snprintf(err, err_len,
                 "old_string must match exactly once; found %zu matches",
                 matches);
        free(content);
        return CNTX_ERR_AMBIGUOUS;
    }

    size_t new_len = strlen(new_text);
    if (new_len > SIZE_MAX - (clen - old_len) - 1) {
        free(content);
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    size_t out_len = clen - old_len + new_len;
    char *out = malloc(out_len + 1);
    if (out == NULL) {
        free(content);
        return CNTX_ERR_IO;
    }
    memcpy(out, content, before);
    memcpy(out + before, new_text, new_len);
    memcpy(out + before + new_len, content + before + old_len,
           clen - before - old_len);
    free(content);

    int rc = write_truncate(path, out, out_len);
    free(out);
    return rc;
}

/* ---- Bounded command execution ---- */

static int64_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static int make_temp_fd(char *pathbuf, size_t len) {
    const char *tmpdir = getenv("TMPDIR");
    if (tmpdir == NULL || tmpdir[0] == '\0') {
        tmpdir = "/tmp";
    }
    snprintf(pathbuf, len, "%s/cntx-cmd-XXXXXX", tmpdir);
    return mkstemp(pathbuf);
}

/* Copy as much of the temp file as fits; append a marker when truncated. */
static void read_bounded_fd(int fd, char *buf, size_t buf_len) {
    if (buf_len == 0) {
        return;
    }
    buf[0] = '\0';
    off_t size = lseek(fd, 0, SEEK_END);
    if (size < 0) {
        return;
    }
    lseek(fd, 0, SEEK_SET);
    size_t to_read = buf_len - 1;
    if (to_read > cntx_tool_read_limit()) to_read = cntx_tool_read_limit();
    size_t total = 0;
    while (total < to_read) {
        ssize_t n = read(fd, buf + total, to_read - total);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            break;
        }
        if (n == 0) {
            break;
        }
        total += (size_t)n;
    }
    buf[total] = '\0';
    if ((off_t)total < size) {
        static const char marker[] = "\n[Output truncated; use a narrower query.]";
        size_t mlen = sizeof(marker) - 1;
        if (total + mlen >= buf_len) {
            mlen = buf_len - 1 - total; /* best effort within bounds */
        }
        memcpy(buf + total, marker, mlen);
        buf[total + mlen] = '\0';
    }
}

static void drain_str(char *dst, size_t dst_len, const char *src) {
    if (dst == NULL || dst_len == 0) {
        return;
    }
    size_t i = 0;
    for (; src != NULL && src[i] != '\0' && i < dst_len - 1; i++) {
        dst[i] = src[i];
    }
    dst[i] = '\0';
}

int cntx_command_run(const char *command, const char *cwd,
                     uint64_t timeout_ms, const volatile int *cancel,
                     char *stdout_buf, size_t stdout_len, char *stderr_buf,
                     size_t stderr_len, int *exit_code, int *timed_out) {
    if (command == NULL || command[0] == '\0' || stdout_buf == NULL ||
        stdout_len == 0 || stderr_buf == NULL || stderr_len == 0) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    if (exit_code != NULL) {
        *exit_code = -1;
    }
    if (timed_out != NULL) {
        *timed_out = 0;
    }
    drain_str(stdout_buf, stdout_len, NULL);
    drain_str(stderr_buf, stderr_len, NULL);

    char out_path[4096], err_path[4096];
    int out_fd = make_temp_fd(out_path, sizeof(out_path));
    if (out_fd < 0) {
        return CNTX_ERR_IO;
    }
    int err_fd = make_temp_fd(err_path, sizeof(err_path));
    if (err_fd < 0) {
        close(out_fd);
        unlink(out_path);
        return CNTX_ERR_IO;
    }

    pid_t pid = fork();
    if (pid < 0) {
        close(out_fd);
        close(err_fd);
        unlink(out_path);
        unlink(err_path);
        return CNTX_ERR_IO;
    }
    if (pid == 0) {
        /* Child: own process group, null stdin, output to temp files. */
        setpgid(0, 0);
        struct rlimit cap = {cntx_tool_command_cap_bytes(), cntx_tool_command_cap_bytes()};
        if (setrlimit(RLIMIT_FSIZE, &cap) != 0) _exit(127);
        int devnull = open("/dev/null", O_RDONLY);
        if (devnull >= 0) {
            dup2(devnull, STDIN_FILENO);
            if (devnull != STDIN_FILENO) {
                close(devnull);
            }
        }
        dup2(out_fd, STDOUT_FILENO);
        dup2(err_fd, STDERR_FILENO);
        if (out_fd != STDOUT_FILENO) {
            close(out_fd);
        }
        if (err_fd != STDERR_FILENO) {
            close(err_fd);
        }
        if (cwd != NULL && chdir(cwd) != 0) {
            _exit(127);
        }
        execl("/bin/sh", "sh", "-c", command, (char *)NULL);
        _exit(127);
    }

    /* Parent: become the process group owner's watcher. */
    setpgid(pid, pid); /* also from the parent, in case the child lost the race */

    int64_t deadline = now_ms() + (int64_t)timeout_ms;
    int status = 0;
    int done = 0;
    int killed = 0;
    while (!done) {
        pid_t r = waitpid(pid, &status, WNOHANG);
        if (r == pid) {
            done = 1;
            break;
        }
        if (r < 0 && errno != EINTR) {
            done = 1;
            break;
        }
        int64_t now = now_ms();
        if (cancel != NULL && __atomic_load_n(cancel, __ATOMIC_RELAXED)) {
            killed = 1;
        } else if (now >= deadline) {
            killed = 1;
            if (timed_out != NULL) {
                *timed_out = 1;
            }
        } else {
            /* Enforce the temporary-file cap while the child runs. */
            struct stat sb;
            if (fstat(out_fd, &sb) == 0 &&
                sb.st_size > (off_t)cntx_tool_command_cap_bytes()) {
                killed = 1;
            }
            if (!killed && fstat(err_fd, &sb) == 0 &&
                sb.st_size > (off_t)cntx_tool_command_cap_bytes()) {
                killed = 1;
            }
        }
        if (killed && !done) {
            /* Terminate the whole process group, then the direct child. */
            kill(-pid, SIGKILL);
            kill(pid, SIGKILL);
            /* Reap; the SIGKILL guarantees this loop terminates. */
            for (int i = 0; i < 100; i++) {
                pid_t w = waitpid(pid, &status, WNOHANG);
                if (w == pid) {
                    done = 1;
                    break;
                }
                struct timespec ts = {0, 10 * 1000 * 1000};
                nanosleep(&ts, NULL);
            }
        }
        if (!done) {
            struct timespec ts = {0, 100 * 1000 * 1000};
            nanosleep(&ts, NULL);
        }
    }

    /* A shell may exit with background children still writing. */
    kill(-pid, SIGKILL);

    read_bounded_fd(out_fd, stdout_buf, stdout_len);
    read_bounded_fd(err_fd, stderr_buf, stderr_len);
    close(out_fd);
    close(err_fd);
    unlink(out_path);
    unlink(err_path);

    if (exit_code != NULL) {
        if (killed) {
            *exit_code = -1;
        } else if (WIFEXITED(status)) {
            *exit_code = WEXITSTATUS(status);
        } else if (WIFSIGNALED(status)) {
            *exit_code = 128 + WTERMSIG(status);
        } else {
            *exit_code = -1;
        }
    }
    return CNTX_OK;
}
