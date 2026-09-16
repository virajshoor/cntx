/* Approval mode semantics: the single source of truth for permission
 * decisions across interactive sessions, one-shot prompts, apply mode, and
 * counsel mode. */
#include "cntx.h"

#include <string.h>

/* The exact decision table from the product contract:
 *
 * | Operation                    | auto | all | manual | file-only | counsel |
 * | Explicit read/glob/grep tool | Allow| Allow| Ask   | Allow     | Allow   |
 * | In-root write/edit/apply     | Ask  | Allow| Ask   | Allow     | Ask     |
 * | Shell command                | Ask  | Allow| Ask   | Deny      | Ask     |
 * | Outside-root direct write    | Deny | Deny | Deny  | Deny      | Deny    |
 *
 * Outside-root denial is layered on top of the mode decision by the sandbox
 * (containment check in Rust); this table only expresses the mode policy.
 */
cntx_decision_t cntx_permission_decide(int mode, int operation) {
    if (mode < CNTX_MODE_AUTO_APPROVE || mode > CNTX_MODE_FILE_ONLY ||
        operation < CNTX_OP_READ || operation > CNTX_OP_NETWORK) {
        return CNTX_DECISION_DENY;
    }

    switch (mode) {
    case CNTX_MODE_ALL_APPROVE:
        return CNTX_DECISION_ALLOW;
    case CNTX_MODE_MANUAL_APPROVE:
        return CNTX_DECISION_ASK;
    case CNTX_MODE_FILE_ONLY:
        switch (operation) {
        case CNTX_OP_READ:
        case CNTX_OP_WRITE:
            return CNTX_DECISION_ALLOW;
        default: /* shell, network */
            return CNTX_DECISION_DENY;
        }
    case CNTX_MODE_AUTO_APPROVE:
    case CNTX_MODE_COUNSEL:
    default:
        switch (operation) {
        case CNTX_OP_READ:
            return CNTX_DECISION_ALLOW;
        default: /* write, shell, network */
            return CNTX_DECISION_ASK;
        }
    }
}

const char *cntx_mode_canonical_name(int mode) {
    switch (mode) {
    case CNTX_MODE_AUTO_APPROVE:
        return "auto-approve";
    case CNTX_MODE_COUNSEL:
        return "counsel";
    case CNTX_MODE_ALL_APPROVE:
        return "all-approve";
    case CNTX_MODE_MANUAL_APPROVE:
        return "manual-approve";
    case CNTX_MODE_FILE_ONLY:
        return "file-only";
    default:
        return NULL;
    }
}

const char *cntx_mode_description(int mode) {
    switch (mode) {
    case CNTX_MODE_AUTO_APPROVE:
        return "allow reads and require approval for writes or shell actions";
    case CNTX_MODE_COUNSEL:
        return "use token-efficient model counsel; same approval behavior as auto-approve";
    case CNTX_MODE_ALL_APPROVE:
        return "execute permitted tools without prompts; file containment still applies";
    case CNTX_MODE_MANUAL_APPROVE:
        return "ask before every tool, file, or shell operation";
    case CNTX_MODE_FILE_ONLY:
        return "allow file reads/writes, but block shell and network";
    default:
        return NULL;
    }
}

int cntx_mode_parse(const char *name, int *out_mode) {
    if (name == NULL || out_mode == NULL) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    if (strcmp(name, "auto-approve") == 0 || strcmp(name, "auto") == 0) {
        *out_mode = CNTX_MODE_AUTO_APPROVE;
    } else if (strcmp(name, "counsel") == 0) {
        *out_mode = CNTX_MODE_COUNSEL;
    } else if (strcmp(name, "all-approve") == 0 || strcmp(name, "allow") == 0) {
        *out_mode = CNTX_MODE_ALL_APPROVE;
    } else if (strcmp(name, "manual-approve") == 0 ||
               strcmp(name, "request-permission") == 0) {
        *out_mode = CNTX_MODE_MANUAL_APPROVE;
    } else if (strcmp(name, "file-only") == 0) {
        *out_mode = CNTX_MODE_FILE_ONLY;
    } else {
        return CNTX_ERR_NOT_FOUND;
    }
    return CNTX_OK;
}

int cntx_mode_next(int mode) {
    switch (mode) {
    case CNTX_MODE_AUTO_APPROVE:
        return CNTX_MODE_ALL_APPROVE;
    case CNTX_MODE_ALL_APPROVE:
        return CNTX_MODE_MANUAL_APPROVE;
    case CNTX_MODE_MANUAL_APPROVE:
        return CNTX_MODE_AUTO_APPROVE;
    /* Legacy extra modes cycle back to the canonical default. */
    case CNTX_MODE_COUNSEL:
    case CNTX_MODE_FILE_ONLY:
    default:
        return CNTX_MODE_AUTO_APPROVE;
    }
}
