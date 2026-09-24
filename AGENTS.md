# Commit and Change Control

## CONTRIBUTING.md Is Authoritative

Before making any changes, the agent MUST read and follow:

```text
CONTRIBUTING.md
```

`CONTRIBUTING.md` defines the project's AI-only development workflow and takes precedence over assumptions made by the agent about normal Git workflows.

RustVim's intended workflow is:

```text
Human
  ↓
prompt / requirement
  ↓
AI
  ↓
implementation
  ↓
AI
  ↓
tests
  ↓
AI
  ↓
commit
  ↓
AI
  ↓
pull request
  ↓
AI
  ↓
review
  ↓
AI
  ↓
merge
  ↓
Human
  ↓
git push
```

Humans are push-only contributors.

The agent MUST NOT assume that a human-authored commit, manually edited patch, manually authored PR, or manual code review is an acceptable part of the normal RustVim development workflow.

---

## Protect Human Changes

AI agents MUST NOT overwrite, discard, reset, revert, or otherwise destroy changes that were already present before the agent started working.

Before modifying the repository, inspect:

```bash
git status --short
git diff --stat
git diff
```

If there are pre-existing changes, treat them as protected human/worktree changes.

The agent MUST:

* preserve unrelated pre-existing modifications;
* avoid formatting unrelated files;
* avoid resetting the working tree;
* avoid `git reset --hard`;
* avoid `git checkout -- <file>`;
* avoid `git restore <file>`;
* avoid deleting untracked files that were present before the task;
* avoid rewriting commits that existed before the task.

If the requested task conflicts with existing uncommitted changes, stop and ask the user before proceeding.

### Important distinction

"Automatically roll back changes made by the AI" means:

> If the AI needs to abandon its own work, it may revert only the changes introduced by that AI during the current task.

It does **not** mean:

> Reset the repository to HEAD and destroy everything currently in the working tree.

Never use a blanket reset as an "undo" mechanism.

---

## AI Change Boundary

At the beginning of a task, establish the repository baseline:

```bash
git status --short
git diff --binary
git ls-files --others --exclude-standard
```

Record the baseline conceptually as:

```text
BASELINE = repository state before this agent's modifications
```

Every modification made by the agent must be attributable to the current task.

The agent must keep track of:

* files it created;
* files it modified;
* files it deleted;
* staged changes it introduced;
* commits it created.

Before finishing, compare the resulting state against the baseline.

The final diff must contain only changes required for the current task.

---

## Automatic Rollback of AI Changes

If the agent decides that its implementation is invalid, unnecessary, or should be abandoned, it MUST roll back its own changes instead of leaving partial work in the working tree.

Rollback must be scoped to the AI's change set.

### Preferred rollback strategy

If the agent has not committed yet:

```text
restore only files/lines changed by the agent
```

Do NOT run:

```bash
git reset --hard
git clean -fd
```

unless the user explicitly requests destruction of the entire working tree.

If the agent created a commit that must be undone, create a new rollback commit rather than rewriting shared history.

The rollback commit MUST use:

```text
rollback(<scope>): revert <previous change>
```

The `rollback` type is explicitly supported by the project's commit convention.

---

## Never Destroy Human Changes

The following commands are prohibited during normal agent operation:

```bash
git reset --hard
git clean -fd
git clean -fdx
git checkout -- .
git restore .
```

They can destroy work that existed before the agent started.

If one of these commands appears necessary, stop and ask the user for explicit permission.

Even explicit permission should be interpreted narrowly: destroy only what the user explicitly identified.

---

# Commit Messages

All commits created by an AI agent MUST follow the project's commit-message convention.

The canonical format is:

```text
<type>(<scope>): <subject>
```

The subject MUST:

* use imperative mood;
* be a complete sentence;
* describe the actual change;
* avoid vague wording.

The scope MUST:

* be short;
* identify the affected part of the project.

Examples:

```text
feat(auth): add JWT refresh token support
fix(api): handle null response from users endpoint
chore(releases): bump version to 2.8.1
```

Bad examples:

```text
fixed stuff
Update auth
Added new feature for users
```

Do not use vague commit messages such as:

```text
changes
fix
update
stuff
work
WIP
misc
small fixes
```

---

## Allowed Commit Types

Use one of the project's defined types:

| Type       | Use                                               |
| ---------- | ------------------------------------------------- |
| `feat`     | New functionality                                 |
| `fix`      | Bug fix                                           |
| `refactor` | Code restructuring without changing functionality |
| `chore`    | General non-code changes                          |
| `build`    | Build-system changes                              |
| `style`    | Code formatting/style changes                     |
| `docs`     | Documentation changes                             |
| `test`     | Test changes                                      |
| `perf`     | Performance improvements                          |
| `ci`       | CI/CD changes                                     |
| `merge`    | Branch/Pull Request merge                         |
| `rollback` | Reverting a previous change                       |

Do not invent new commit types unless the project's contribution rules are explicitly updated.

---

## CHANGELOG Commit Rule

If `CHANGELOG.md` was updated as part of a release/version change, use:

```text
chore(releases): bump version to <version>
```

Do not replace this with a generic commit message such as:

```text
docs(changelog): update changelog
```

when the change represents the documented version bump.

---

## One Logical Change Per Commit

Prefer one logical change per commit.

For example:

```text
feat(editor): add syntax highlighting
test(editor): cover syntax highlighting
```

may be appropriate when the project workflow calls for separate commits.

However, do not artificially split a tightly coupled implementation and its tests when they form one inseparable change.

The commit should leave the repository in a coherent state.

Never create commits containing unrelated changes merely because they are present in the working tree.

---

## Commit Before Commit

Before creating a commit:

```bash
git status --short
git diff
git diff --cached
```

Verify that:

1. every staged file belongs to the current task;
2. no human/pre-existing changes were staged;
3. no secrets are included;
4. generated junk is not included;
5. tests and documentation are consistent with the change.

Then create the commit.

The agent MUST NOT blindly run:

```bash
git add .
git commit -m "..."
```

because `git add .` may stage unrelated human changes.

Stage only the files required for the current change.

---

## Commit Validation

Before committing, run the relevant project checks.

For normal Rust changes:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

If any required check fails, do not create a normal "successful" implementation commit.

Fix the issue first or roll back the AI's changes.

---

## Existing Commits

Do not rewrite existing project history unless explicitly requested.

Avoid:

```bash
git rebase
git commit --amend
git reset
git filter-repo
```

when the operation would modify commits that predate the current task.

RustVim's contribution model expects AI-generated commits and allows humans to push the resulting repository state. Preserve existing history.

---

## Rollback Commits

When an AI-generated change has already been committed and must be undone, prefer a new rollback commit.

Format:

```text
rollback(<scope>): revert <previous change>
```

Example:

```text
rollback(editor): revert unstable syntax highlighting
```

Do not silently rewrite history to hide the original AI-generated change.

The history should make the development process understandable.

---

# Pull Requests

According to `CONTRIBUTING.md`, pull requests are AI-generated.

An AI-created PR should include:

* description;
* implementation details;
* test results;
* known problems;
* explanation of why the change is necessary.

The agent should not claim that a human reviewed or approved an AI-generated change unless that actually occurred.

---

# Code Review

Code review is part of the AI-only workflow described by `CONTRIBUTING.md`.

An agent reviewing another AI's work should:

1. inspect the diff;
2. inspect affected code;
3. run relevant tests;
4. check architectural consistency;
5. check security implications;
6. check documentation;
7. report concrete problems.

Do not approve code merely because tests pass.

Do not reject code based on subjective stylistic preferences when the existing project conventions already support the implementation.

---

# Final Repository Check

Before finishing a task:

```bash
git status --short
git diff --check
git diff
git log -1 --oneline
```

Verify:

```text
✓ only task-related files changed
✓ pre-existing human changes preserved
✓ no accidental deletions
✓ no secrets
✓ no unrelated formatting
✓ tests pass
✓ clippy passes
✓ formatting passes
✓ commit follows <type>(<scope>): <subject>
✓ CHANGELOG rule is respected
✓ CONTRIBUTING.md rules are respected
```

If the AI cannot guarantee that its changes are isolated from pre-existing work, it must stop rather than risk destroying or committing somebody else's changes.

---

# Agent Safety Rule

The most important rule for repository modification is:

> Never trade convenience for data loss.

A clean working tree is not more important than preserving existing work.

If rollback is required, roll back the AI's changes — not the repository.

If ownership of a change is ambiguous, do not delete it, reset it, or commit it. Ask the user.

