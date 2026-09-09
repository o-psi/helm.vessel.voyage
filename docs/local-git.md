# Git and GitHub

The repository is [o-psi/voyage](https://github.com/o-psi/voyage). Use ordinary Git
in normal clones and linked worktrees.

This particular workspace reserves `.git`; its metadata is under
`.local-git/worktree.git`. Use the existing wrapper instead of initializing another
repository:

```sh
./scripts/local-git status --short --branch
./scripts/local-git diff
./scripts/local-git fetch origin
./scripts/local-git diff --check
./scripts/local-git add path/to/your/change
./scripts/local-git commit -m "Describe the change"
```

If the wrapper is absent, use its explicit invocation against the **existing**
metadata from the checkout root (not ordinary Git discovery):

```sh
git --git-dir=.local-git/worktree.git --work-tree=. status --short --branch
git --git-dir=.local-git/worktree.git --work-tree=. diff
git --git-dir=.local-git/worktree.git --work-tree=. diff --cached
```

The same prefix supports normal add/commit/fetch/merge/push commands. A missing
wrapper does not authorize restoring deleted scripts or initializing metadata.
Inspect both index and worktree changes; path-scoped commits must not include
someone else's staged edits.

Inspect status/diffs before editing and stage only the authorized change. Do not
reset, clean, force-push or overwrite concurrent work. Preserve unfinished branches,
worktrees, release artifacts and evidence. A documentation rewrite does not authorize
removing private runtime data or repository metadata.

Use the explicit repository with GitHub CLI when autodetection is inappropriate:

```sh
gh issue list --repo o-psi/voyage --state all
gh issue view 77 --repo o-psi/voyage
```

Follow [project instructions](../AGENTS.md) for scope tracking, validation and
publication. Unless directed otherwise, deliver completed work to local `main`,
push normally to `origin/main`, and verify local/remote commit equality. If access,
conflicts, ongoing checks or branch protection block delivery, report the actual
state rather than bypassing the blocker. A local Forgejo service is not required.
