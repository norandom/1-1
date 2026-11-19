# Agentic SDLC and Spec-Driven Development

Kiro-style Spec-Driven Development on an agentic SDLC

## Project Memory
Project memory keeps persistent guidance (steering, specs notes, component docs) so OpenCode honors your standards each run. Treat it as the long-lived source of truth for patterns, conventions, and decisions.

- Use `.kiro/steering/` for project-wide policies: architecture principles, naming schemes, security constraints, tech stack decisions, api standards, etc.
- Use local `AGENTS.md` files for feature or library context (e.g. `src/lib/payments/AGENTS.md`): describe domain assumptions, API contracts, or testing conventions specific to that folder. OpenCode auto-loads these when working in the matching path.
- Specs notes stay with each spec (under `.kiro/specs/`) to guide specification-level workflows.

## Project Context

### Paths
- Steering: `.kiro/steering/`
- Specs: `.kiro/specs/`

### Steering vs Specification

**Steering** (`.kiro/steering/`) - Guide AI with project-wide rules and context
**Specs** (`.kiro/specs/`) - Formalize development process for individual features

### Active Specifications
- Check `.kiro/specs/` for active specifications
- Use `/kiro-spec-status [feature-name]` to check progress

## Development Guidelines
- Think in English, generate responses in English. All Markdown content written to project files (e.g., requirements.md, design.md, tasks.md, research.md, validation reports) MUST be written in the target language configured for this specification (see spec.json.language).

## Minimal Workflow
- Phase 0 (optional): `/kiro-steering`, `/kiro-steering-custom`
- Discovery: `/kiro-discovery "idea"` — determines action path, writes brief.md + roadmap.md for multi-spec projects
- Phase 1 (Specification):
  - Single spec: `/kiro-spec-quick {feature} [--auto]` or step by step:
    - `/kiro-spec-init "description"`
    - `/kiro-spec-requirements {feature}`
    - `/kiro-validate-gap {feature}` (optional: for existing codebase)
    - `/kiro-spec-design {feature} [-y]`
    - `/kiro-validate-design {feature}` (optional: design review)
    - `/kiro-spec-tasks {feature} [-y]`
  - Multi-spec: `/kiro-spec-batch` — creates all specs from roadmap.md in parallel by dependency wave
- Phase 2 (Implementation): `/kiro-impl {feature} [tasks] [--review required|inline|off]`
  - Without task numbers: autonomous mode (subagent per task + independent review + final validation)
  - With task numbers: manual mode (selected tasks in main context, still reviewer-gated before completion)
  - `--review off` skips task-local review; use it intentionally and keep `/kiro-validate-impl {feature}` as the final quality gate
  - `/kiro-validate-impl {feature}` (standalone re-validation)
- Progress check: `/kiro-spec-status {feature}` (use anytime)

## Skills Structure
Skills are located in `.opencode/skills/kiro-*/SKILL.md`
- Each skill is a directory with a `SKILL.md` file
- Use `/skills` to inspect currently available skills
- Invoke a skill directly with `/kiro-<skill-name>`
- Use skills explicitly requested by the user and skills relevant to the task's domain, including design, accessibility, and UX.
- Select skills from their descriptions or metadata first, then read only the selected skills and the references needed for the task.
- Follow explicit host and project rules and retain required workflow checks. Do not skip relevant skills just because the task is small.
- `kiro-review` — task-local adversarial review protocol used by reviewer subagents
- `kiro-debug` — root-cause-first debug protocol used by debugger subagents
- `kiro-verify-completion` — fresh-evidence gate before success or completion claims

## Development Rules
- 3-phase approval workflow: Requirements → Design → Tasks → Implementation
- Human review required each phase; use `-y` only for intentional fast-track
- Keep steering current and verify alignment with `/kiro-spec-status`
- Follow the user's instructions precisely, and within that scope act autonomously: gather the necessary context and complete the requested work end-to-end in this run, asking questions only when essential information is missing or the instructions are critically ambiguous.

## Steering Configuration
- For spec and implementation work, load the core steering files below from `.kiro/steering/`. Reuse current context rather than rereading unchanged files.
- Load additional steering only when required by project rules or relevant to the task.
- Default files: `product.md`, `tech.md`, `structure.md`
- Custom files are supported (managed via `/kiro-steering-custom`)


## Windows execution and CI policy
- On Windows use native PowerShell/pwsh and native executables. Do not run repository commands through wsl.exe, a WSL distribution shell, Git Bash, MSYS/Cygwin bash, or wrappers that delegate to them. Inspect ambiguous wrappers before execution; cmd.exe is allowed only as native CLI plumbing when needed.
- A Podman engine backed by WSL is allowed. Invoke it through native Podman/Docker/Dagger clients; Linux commands inside managed CI containers are allowed. This exception does not authorize direct WSL shell commands, podman machine ssh for repository work, or moving the repository/build into WSL.
- Docker and Docker Compose native clients may be used where already available; do not add Compose merely to run Dagger.
- Run from the relevant repository. Honor configured caches; put Windows disposable outputs under D:/BuildArtifacts/<tool-or-project>, never a drive root. Linux engine caches stay inside the engine filesystem. Same-directory atomic staging of persisted application data is part of the storage transaction, not a disposable build cache.
- Rust test tiers use cargo nextest run plus cargo test --doc. Nextest does not replace doctests. Keep independent test data and serialize genuine shared OS resources.
- Target CI policy: Dagger owns portable/Linux check and build logic; GitHub Actions triggers it and collects artifacts. Native Windows MSVC/WebView2 and macOS bundle/UI jobs remain explicit exceptions using shared tier commands. Do not claim Linux container checks validate native Windows/macOS artifacts.
- Windows Actions jobs use shell: pwsh. Pin toolchain/engine/action versions; do not require Dagger Cloud secrets for PR checks.

## Local build storage
- User instruction: no build-blocking guard or mandatory size checkpoints. Occasional explicit cleanup is sufficient.
- Honor CARGO_TARGET_DIR and reuse existing build directories. Optional usage report: scripts/Invoke-BuildStorage.ps1. Cleanup while idle: -Action CleanProject; uses cargo clean --package rdp-client only.
- Preserve source, user data, session history, evidence and active output. Keep package dev/test debug=0 and incremental=false by default. Do not clean other projects without ownership review.
