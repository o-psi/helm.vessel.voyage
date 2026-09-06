# Installer and Linux user service

`voyage-installer` with no arguments runs the simulated setup wizard. Its answers
stay in memory and do not provision providers, remote access or services.

The explicit `install-user-service` command performs a real local installation
from an already built or extracted full release. It requires Linux, an ordinary
user account and three executable files: `helm`, `vessel` and `voyage`.

```sh
/absolute/release/bin/voyage-installer install-user-service --bin-dir /absolute/release/bin --dry-run
/absolute/release/bin/voyage-installer install-user-service --bin-dir /absolute/release/bin --start
```

Omit `--start` to install files without calling the service manager. `--dry-run`
validates input paths and prints the proposed unit without changing any files.
The installer rejects symlinks, group/other-writable installation ancestors,
unexpected owners, elevated execution and existing service units. It does not
download executables or collect credentials.

Each installation keeps executable copies in a unique directory beneath
`~/.local/share/voyage/releases`. The output prints that directory; its Helm
executable can be run directly. The unit is
`~/.config/systemd/user/voyage-vessel.service`. Vessel's private state directory is
`~/.local/state/voyage/vessel`; its client socket is `vessel.sock` there. The unit
starts `vessel local-serve` with capacity 16 and an absolute voyage executable path.
Vessel and voyage use the executing user's configuration and credentials, never
the operator interface's forwarded credentials.

`--start` reloads the user manager, enables and starts the unit, then checks that
it is active. An activation failure retains the installation and reports how to
inspect and retry it. A successful active check establishes supervisor activation,
not provider readiness or successful session execution.

```sh
voyage-installer service-status
voyage-installer service-stop
systemctl --user start voyage-vessel.service
```

The service uses `KillMode=process`: stopping or restarting the supervisor does
not terminate its independent voyage processes. `service-stop` first checks the
effective systemd kill mode and refuses if an override would kill descendants.
Drain individual voyages through Vessel before stopping when that is desired.
Supervisor status does not establish whether an individual voyage survived.

Service lifetime follows the systemd user manager. Boot and logout persistence
require separately configured user lingering; the installer never enables it or
elevates privileges. Machine reboot still terminates processes. Native reboot,
logout and recovery deployment must be verified on the actual host.

After stopping the supervisor, remove its unit with:

```sh
voyage-installer service-uninstall
```

Uninstall refuses an active supervisor, disables the unit and removes its file.
It retains versioned binaries, runtime state, configuration and credentials, and
does not kill surviving voyages. An explicit upgrade can uninstall the stopped
unit and install from another release. Keep the previous binary directory for
rollback, and do not remove executables that surviving voyages may still use.
Reinstalling does not authorize replay of uncertain work or imply process survival.

Service installation and management refuse unsupported platforms. Full Windows
and macOS release archives may include binaries; that is not native service
deployment evidence. The downloaded standalone wizard does not contain the
three runtime binaries required by the installation command.
