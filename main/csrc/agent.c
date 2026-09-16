/* Agent/goal state machine and step accounting. The Rust host persists the
 * state and injects it into requests; C decides every transition. */
#include "cntx.h"

#include <string.h>

const char *cntx_goal_status_name(int status) {
    switch (status) {
    case CNTX_GOAL_NONE:
        return "none";
    case CNTX_GOAL_ACTIVE:
        return "active";
    case CNTX_GOAL_PAUSED:
        return "paused";
    case CNTX_GOAL_BLOCKED:
        return "blocked";
    case CNTX_GOAL_COMPLETED:
        return "completed";
    case CNTX_GOAL_CANCELLED:
        return "cancelled";
    default:
        return NULL;
    }
}

int cntx_goal_parse_status(const char *name, int *out_status) {
    if (name == NULL || out_status == NULL) {
        return CNTX_ERR_INVALID_ARGUMENT;
    }
    if (strcmp(name, "none") == 0) {
        *out_status = CNTX_GOAL_NONE;
    } else if (strcmp(name, "active") == 0) {
        *out_status = CNTX_GOAL_ACTIVE;
    } else if (strcmp(name, "paused") == 0) {
        *out_status = CNTX_GOAL_PAUSED;
    } else if (strcmp(name, "blocked") == 0) {
        *out_status = CNTX_GOAL_BLOCKED;
    } else if (strcmp(name, "completed") == 0) {
        *out_status = CNTX_GOAL_COMPLETED;
    } else if (strcmp(name, "cancelled") == 0) {
        *out_status = CNTX_GOAL_CANCELLED;
    } else {
        return CNTX_ERR_NOT_FOUND;
    }
    return CNTX_OK;
}

/* Invalid transitions leave the status unchanged. */
cntx_goal_status_t cntx_goal_transition(int status, int event) {
    switch (event) {
    case CNTX_GOAL_EVENT_START:
        /* Starting a new goal is allowed from none, completed, and
         * cancelled. An existing active/paused/blocked goal must be
         * cancelled first; keep it and let the caller report that. */
        if (status == CNTX_GOAL_NONE || status == CNTX_GOAL_COMPLETED ||
            status == CNTX_GOAL_CANCELLED) {
            return CNTX_GOAL_ACTIVE;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_PAUSE:
        if (status == CNTX_GOAL_ACTIVE) {
            return CNTX_GOAL_PAUSED;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_RESUME:
        if (status == CNTX_GOAL_PAUSED || status == CNTX_GOAL_BLOCKED) {
            return CNTX_GOAL_ACTIVE;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_CANCEL:
        if (status == CNTX_GOAL_ACTIVE || status == CNTX_GOAL_PAUSED ||
            status == CNTX_GOAL_BLOCKED) {
            return CNTX_GOAL_CANCELLED;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_COMPLETE:
        if (status == CNTX_GOAL_ACTIVE) {
            return CNTX_GOAL_COMPLETED;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_BLOCK:
        if (status == CNTX_GOAL_ACTIVE) {
            return CNTX_GOAL_BLOCKED;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_STEP_LIMIT:
        /* Hitting the cap pauses; it never marks the goal complete. */
        if (status == CNTX_GOAL_ACTIVE) {
            return CNTX_GOAL_PAUSED;
        }
        return (cntx_goal_status_t)status;
    case CNTX_GOAL_EVENT_PROVIDER_FAILURE:
    default:
        /* Provider failure preserves state and returns control. */
        return (cntx_goal_status_t)status;
    }
}

int cntx_goal_should_continue(int status, uint32_t steps_used,
                              uint32_t max_steps) {
    if (status != CNTX_GOAL_ACTIVE) {
        return 0;
    }
    if (max_steps == 0) {
        max_steps = cntx_goal_default_max_steps();
    }
    return steps_used < max_steps;
}

int32_t cntx_agent_next(int32_t status, uint32_t used, uint32_t limit,
                        int32_t interrupted, int32_t denied, uint32_t stalled) {
    if (interrupted || denied || stalled >= 2 || used >= limit)
        return CNTX_AGENT_PAUSE;
    if (status != CNTX_GOAL_NONE && status != CNTX_GOAL_ACTIVE)
        return CNTX_AGENT_STOP;
    return CNTX_AGENT_REQUEST;
}
