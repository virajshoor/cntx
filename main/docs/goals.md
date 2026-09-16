# Goals

`/goal` gives Cntx Code persistent, bounded, goal-oriented work. The goal is
stored in the session, survives compaction and reload, and drives the agent
loop until it completes, blocks, pauses, or hits its step budget.

## Command grammar

| Command | Behavior |
| --- | --- |
| `/goal` | Print the objective, status, progress, evidence, and used/remaining steps. No model call. |
| `/goal <objective>` | Persist a new goal and start its run. Replacing an active/paused/blocked goal is rejected; cancel it first. |
| `/goal resume` | Resume paused/blocked work with a fresh batch of up to 50 steps and display that budget. |
| `/goal pause` | Mark an active goal paused. No model call. Ctrl+C is how you interrupt a busy turn. |
| `/goal cancel` | Mark the goal cancelled and keep the transcript. No model call. |
| `/goal new <objective>` | Explicit form for objectives starting with reserved words such as `resume`. |

## How a goal run works

- Every provider turn counts toward the step cap, including text-only
  continuation turns; compaction requests are separately bounded and reported.
- The model reports state through a validated `goal_update` tool action with
  `status` (`active`, `blocked`, or `completed`), `progress`, and `evidence`.
  Paused/cancelled are user controls the model cannot set.
- A `completed` update requires nonempty evidence referencing actual tool
  results or checks from this session, or a reason why no executable check
  applies. Completion is displayed as the model's reported outcome backed by
  that evidence — not an independent proof.
- A plain prose response never completes an active goal. After two consecutive
  turns with no tool action or progress update, the goal pauses with an
  explanation of the stall.
- Reaching the step budget pauses the goal (never marks it complete). Denied
  approval and Ctrl+C also pause. Provider failures preserve state and return
  control. There is no background daemon and no unbounded loop.

## Persistence

The session stores the objective, status, progress, evidence, steps used, and
the step budget. Saving happens atomically after each tool result and state
transition. `/compact` keeps the goal, its progress, and the summary; `/resume`
(and `cntx session resume`) reload them. Sessions belong to the workspace where
they were created — resuming one from another directory shows its path instead
of switching roots silently.
