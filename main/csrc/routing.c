/* Routing classification: prompt-size routing tiers, counsel task
 * classification, and OpenCode Go protocol selection. */
#include "cntx.h"

#include <string.h>
#include <strings.h>
#include <stdlib.h>
#include <math.h>

cntx_route_size_t cntx_route_classify(size_t estimated_tokens,
                                      size_t small_threshold,
                                      size_t medium_threshold) {
    if (estimated_tokens <= small_threshold) {
        return CNTX_ROUTE_SMALL;
    }
    if (estimated_tokens <= medium_threshold) {
        return CNTX_ROUTE_MEDIUM;
    }
    return CNTX_ROUTE_LARGE;
}

cntx_task_t cntx_counsel_classify(const char *prompt) {
    if (prompt == NULL) {
        return CNTX_TASK_EVALUATE;
    }
    static const char *const refactor_words[] = {
        "refactor", "restructure", "rewrite", "architecture",
        "extract",  "split",       "modularize", "redesign",
    };
    static const char *const small_words[] = {
        "fix", "change", "add", "implement", "update",
        "modify", "patch", "build",
    };
    /* Bounded scan: counsel classification reads a fixed prefix so a huge
     * prompt cannot make classification expensive. */
    size_t limit = 4096;
    size_t len = strlen(prompt);
    if (len > limit) {
        len = limit;
    }

    /* Case-insensitive substring scan over the bounded prefix. */
    char head[4097];
    memcpy(head, prompt, len);
    head[len] = '\0';
    for (char *p = head; *p != '\0'; p++) {
        *p = (char)((*p >= 'A' && *p <= 'Z') ? *p - 'A' + 'a' : *p);
    }
    for (size_t i = 0; i < sizeof(refactor_words) / sizeof(char *); i++) {
        if (strstr(head, refactor_words[i]) != NULL) {
            return CNTX_TASK_REFACTOR;
        }
    }
    for (size_t i = 0; i < sizeof(small_words) / sizeof(char *); i++) {
        if (strstr(head, small_words[i]) != NULL) {
            return CNTX_TASK_SMALL_CHANGE;
        }
    }
    return CNTX_TASK_EVALUATE;
}

/* OpenCode Go model families (verified against the official docs):
 *
 *   /chat/completions (OpenAI-compatible): GLM, Kimi, LongCat, DeepSeek,
 *     MiMo, Hy
 *   /messages (Anthropic-compatible): MiniMax, Qwen
 *   /responses (OpenAI Responses): GPT 5.6 Luna, Grok 4.6, Muse
 *     Spark Contributor
 *
 * Model ids arrive as bare ids; display names like `opencode-go/<id>` are
 * normalized by the Rust adapter before routing. */
cntx_go_protocol_t cntx_go_protocol_for_model(const char *model_id) {
    if (model_id == NULL) {
        return CNTX_GO_PROTOCOL_UNKNOWN;
    }
    /* Lowercase prefix copy, bounded. */
    char head[512];
    size_t len = strlen(model_id);
    if (len >= sizeof(head)) {
        len = sizeof(head) - 1;
    }
    for (size_t i = 0; i < len; i++) {
        char c = model_id[i];
        head[i] = (char)((c >= 'A' && c <= 'Z') ? c - 'A' + 'a' : c);
    }
    head[len] = '\0';

    if (strstr(head, "glm") != NULL || strstr(head, "kimi") != NULL ||
        strstr(head, "longcat") != NULL || strstr(head, "deepseek") != NULL ||
        strstr(head, "mimo") != NULL || strncmp(head, "hy", 2) == 0) {
        return CNTX_GO_PROTOCOL_CHAT;
    }
    if (strstr(head, "minimax") != NULL || strstr(head, "qwen") != NULL) {
        return CNTX_GO_PROTOCOL_MESSAGES;
    }
    if (strstr(head, "gpt") != NULL || strstr(head, "grok") != NULL ||
        strstr(head, "muse") != NULL) {
        return CNTX_GO_PROTOCOL_RESPONSES;
    }
    return CNTX_GO_PROTOCOL_UNKNOWN;
}

static double parameter_billions(const char *value) {
    if (!value) return -1;
    /* Find a numeric suffix such as 20b or 1.6t, not the 'b' in a name. */
    for (const char *p = value; *p; p++) {
        if (*p < '0' || *p > '9') continue;
        char *end = NULL;
        double size = strtod(p, &end);
        if (isfinite(size) && size > 0 && (*end == 'b' || *end == 't'))
            return size * (*end == 't' ? 1000 : 1);
        if (end > p) p = end - 1;
    }
    return -1;
}

int32_t cntx_model_rank(int32_t provider, const char *id, const char *usage,
                       const char *parameter_size) {
    if (!id) return 3;
    if (provider == 0) {
        if (strstr(id, "haiku")) return 0;
        if (strstr(id, "sonnet")) return 1;
        if (strstr(id, "opus")) return 2;
        return 3;
    }
    if (provider == 1) {
        if (strstr(id, "nano") || strstr(id, "mini") || strstr(id, "small")) return 0;
        if (strstr(id, "pro") || strstr(id, "large") || strstr(id, "max")) return 2;
        return 1;
    }
    if (provider != 2 && provider != 3) return 3;
    if (usage) {
        if (strstr(usage, "extra") || strstr(usage, "high")) return 2;
        if (strstr(usage, "medium")) return 1;
        if (strstr(usage, "low") || strstr(usage, "light") || strstr(usage, "small")) return 0;
    }
    if (strstr(id, "pro") || strstr(id, "ultra") || strstr(id, "max") ||
        strstr(id, "extra-high") || strstr(id, "extra_high")) return 2;
    if (provider == 3 && (strstr(id, "flash") || strstr(id, "mini") || strstr(id, "light"))) return 0;
    double size = parameter_billions(parameter_size);
    if (size < 0) size = parameter_billions(id);
    if (size < 0) return 3;
    if (size <= (provider == 3 ? 25 : 10)) return 0;
    return size < 40 ? 1 : 2;
}

static int newer(const cntx_model_candidate *a, const cntx_model_candidate *b) {
    return a->created > b->created || (a->created == b->created && strcmp(a->id, b->id) > 0);
}

int64_t cntx_model_select(const cntx_model_candidate *models, size_t count,
                          int32_t target_rank, const char *default_id, int32_t *reason) {
    if ((!models && count) || !reason || count > INT64_MAX) return -2;
    *reason = 2;
    for (size_t i = 0; i < count; i++) if (!models[i].id) return -2;
    if (count == 1) { *reason = 0; return 0; }
    if (!count) {
        if (default_id) { *reason = 1; return -1; }
        return -2;
    }
    int64_t target = -1, any = 0;
    for (size_t i = 0; i < count; i++) {
        if (default_id && strcmp(models[i].id, default_id) == 0) { *reason = 1; return (int64_t)i; }
        if (models[i].rank == target_rank && (target < 0 || newer(&models[i], &models[target]))) target = (int64_t)i;
        if (newer(&models[i], &models[any])) any = (int64_t)i;
    }
    return target >= 0 ? target : any;
}
