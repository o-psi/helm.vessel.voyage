# Local Git server

Voyage includes a loopback-only Git server for development without an external forge.
Its canonical bare repository and working-copy metadata live in `.local-git/`, which is
ignored by the project.

Start the server in a dedicated terminal:

```sh
./scripts/local-git-server
```

The remote URL is `git://127.0.0.1:9418/voyage.git`. Since this environment reserves
the workspace's `.git` path, use the transparent helper for working-copy operations:

```sh
./scripts/local-git status
./scripts/local-git log --oneline
./scripts/local-git add path/to/file
./scripts/local-git commit -m "Describe the change"
./scripts/local-git push origin main
```

Clone it elsewhere normally:

```sh
git clone git://127.0.0.1:9418/voyage.git
```

The daemon intentionally listens only on loopback. `git daemon` has no authentication
or transport encryption, so do not change it to `0.0.0.0`. For LAN or internet access,
put the bare repository behind SSH or deploy Forgejo with TLS and authentication.
