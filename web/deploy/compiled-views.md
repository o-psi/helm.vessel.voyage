# Compiled-view permissions on Helm Web deployments

The former laptop deployment served this checkout with a separate runtime storage
path and PHP-FPM worker `http`. It is no longer the public Helm Web origin. The
current origin is Proxmox CT 106: application source is `/srv/helm/app`, private
runtime state is under `/srv/helm/runtime`, and PHP-FPM runs as `www-data`.

Check the effective Laravel storage and compiled-view paths before any CLI render
or cache command. Do not assume `web/storage` is active: the laptop deployment's
`.env` set `LARAVEL_STORAGE_PATH`, and a CLI bootstrap could write its live cache.

On CT 106, `php artisan optimize` as `helm` cached paths under
`/srv/helm/app/storage` even though `.env` points `LARAVEL_STORAGE_PATH` to the
private runtime. The HTML `/up` route and authenticated console then returned
500 while `/up` with `Accept: application/json` returned 200. The active error
was `touch(): Utime failed: Operation not permitted` in `BladeCompiler`: a
compiled view owned by `helm` cannot have its timestamp set by `www-data`, even
when the file is group writable. Clear the configuration cache to restore the
private runtime path, and let `www-data` create compiled views there. Check that
new compiled files appear under `/srv/helm/runtime/storage/framework/views`,
then verify both the HTML `/up` route and an authenticated console request.

Laravel's atomic file replacement applies `0777 - umask()`. A CLI process with
umask `0077` creates compiled views with mode `0700`, masking inherited ACL
access for the `http` PHP-FPM worker. Repeated authenticated console requests
then fail with `include(...storage/framework/views/...php): Permission denied`.
A default directory ACL alone does not prevent recurrence.

Rendering tests must set `VIEW_COMPILED_PATH` to an isolated temporary directory
before bootstrapping Laravel and remove that directory afterwards. The Flux,
console, owner-connection, profile-menu and sidebar-action rendering checks do
this. The sidebar-action check previously omitted this override and recreated an
unreadable live console view; its render now uses a temporary cache removed in a
`finally` block. For other ad hoc CLI render checks, use an existing private
scratch directory via that environment variable as well. Run deliberate live
cache operations only during controlled CT deployment or recovery, with the
configured `helm` and `www-data` shared access; verify the generated views are
readable by `www-data`.

Do not change the process-wide umask to solve this: it also governs secret files.

For recovery, inspect the active Laravel log and the exact failing generated
file with `namei -l` and `getfacl`. On CT 106 the worker is `www-data`; restore only
its access to affected generated PHP files, including ownership when Laravel
needs to update their timestamps. The laptop's old `http` ACL must not be copied
to the CT.

Do not recursively make storage, credentials, sessions or database files public.
Verify effective access and repeat the authenticated request. A successful public
login page does not verify the authenticated console.
