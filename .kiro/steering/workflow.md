# Workflow Rules

## Hold at task-group boundaries

- During autonomous task execution (e.g. `/kiro-impl`), when a task GROUP in the
  active `tasks.md` is finished (every sub-task of the group marked `[x]`,
  reviewed, and committed), STOP.
- Do NOT dispatch the first task of the next group. Hold and wait for the user
  to clear/compact the session context and explicitly resume.
- Before holding: ensure task checkboxes are updated, per-task commits exist,
  and the working-state checkpoint (mmhmem) is saved so a fresh context can
  resume cleanly at the next group.
