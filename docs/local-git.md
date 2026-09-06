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
