# Git and GitHub workflow

Voyage's repository, issues and pull requests are on
[GitHub](https://github.com/o-psi/voyage). Use normal Git in ordinary clones:

```sh
git clone https://github.com/o-psi/voyage.git
cd voyage
git status --short
git remote -v
```

This special local workspace reserves `.git`. Its working-copy metadata lives in
ignored `.local-git/worktree.git`; use the existing wrapper without initializing
another repository:

```sh
./scripts/local-git status --short
./scripts/local-git remote -v
./scripts/local-git fetch origin
./scripts/local-git diff --check
./scripts/local-git add path/to/your/change
./scripts/local-git commit -m "Describe the change"
```

Inspect status and diffs first. Stage only your changes and preserve concurrent
work. In linked worktrees, normal Git works. Review changes in a focused branch
and GitHub PR; do not reset, clean, force-push or overwrite user changes implicitly.

Use an explicit GitHub repository when the wrapper layout prevents autodetection:

```sh
gh issue view 77 --repo o-psi/voyage
gh pr list --repo o-psi/voyage
```

Follow [the project instructions](../AGENTS.md) for complete paginated issue review,
tracking and verification before substantive work. A local Forgejo server or Git
daemon is not required for this workflow. Local evidence and preserved branch
archives are documented in [the worktree audit](worktree-cleanup.md); they are not
an alternative issue tracker or source of current feature readiness.
