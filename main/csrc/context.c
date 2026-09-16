/* Context budget and retention rules: bounded request assembly decisions
 * shared by every caller. */
#include "cntx.h"
#include <stdlib.h>
#include <string.h>

size_t cntx_context_default_budget(void) { return 16000; }

int cntx_context_should_compact(size_t estimated_tokens, size_t budget) {
    if (budget == 0) {
        budget = cntx_context_default_budget();
    }
    return estimated_tokens > budget;
}

/* Reserve output space from a known context window. Unknown windows keep
 * the default budget. The result never drops below a usable floor so a
 * tiny window produces a clear budget rather than zero room. */
size_t cntx_context_budget_for_window(size_t model_context_window) {
    const size_t reserve = 4096;
    if (model_context_window == 0) return cntx_context_default_budget();
    size_t allowed = model_context_window > reserve ? model_context_window - reserve : 1;
    return allowed < cntx_context_default_budget() ? allowed : cntx_context_default_budget();
}

size_t cntx_context_split(const int32_t *user_turns, size_t count) {
    if (user_turns == NULL) return 0;
    size_t users = 0;
    for (size_t i = count; i > 0; i--) {
        if (user_turns[i - 1] && ++users == 2) return i - 1;
    }
    return 0;
}

size_t cntx_estimate_tokens(size_t characters, size_t words) {
    size_t estimate = characters / 4 + (characters % 4 != 0);
    return estimate > words ? estimate : words;
}

size_t cntx_context_score(const char *content, const char *const *terms, size_t count) {
    if (!content || (!terms && count)) return 0;
    size_t score = 0;
    for (size_t i = 0; i < count; i++)
        if (terms[i] && terms[i][0] && strstr(content, terms[i])) score++;
    return score;
}

int32_t cntx_optimize(const char *const *raw, const char *const *normalized,
                     size_t count, char *out, size_t capacity,
                     size_t *written, size_t *duplicates) {
    if ((!raw && count) || (!normalized && count) || !out || !capacity ||
        !written || !duplicates || count > SIZE_MAX / sizeof(char *))
        return CNTX_ERR_INVALID_ARGUMENT;
    const char **seen = count ? calloc(count, sizeof(char *)) : NULL;
    if (count && !seen) return CNTX_ERR_IO;
    size_t nseen = 0, used = 0;
    int fenced = 0, blank = 0, rc = CNTX_OK;
    *duplicates = 0;
    for (size_t i = 0; i < count; i++) {
        if (!raw[i] || !normalized[i]) { rc = CNTX_ERR_INVALID_ARGUMENT; break; }
        int fence = strncmp(normalized[i], "```", 3) == 0;
        const char *line = fenced || fence ? raw[i] : normalized[i];
        if (fence) fenced = !fenced;
        if (!normalized[i][0]) { if (used) blank = 1; continue; }
        if (!fenced && !fence && strlen(line) > 24) {
            int duplicate = 0;
            /* ponytail: quadratic in line count; use a hash table if large
             * prompts make this measurable. Request budgets bound normal input. */
            for (size_t j = 0; j < nseen; j++) {
                if (strcmp(seen[j], line) == 0) { duplicate = 1; break; }
            }
            if (duplicate) { (*duplicates)++; continue; }
            seen[nseen++] = line;
        }
        size_t len = strlen(line), separators = used ? (size_t)(1 + blank) : 0;
        if (separators >= capacity - used || len >= capacity - used - separators) {
            rc = CNTX_ERR_CAP_EXCEEDED; break;
        }
        while (separators--) out[used++] = '\n';
        memcpy(out + used, line, len);
        used += len;
        blank = 0;
    }
    out[used] = 0;
    *written = used;
    free(seen);
    return rc;
}
