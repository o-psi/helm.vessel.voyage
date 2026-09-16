# Compiled-view permissions on the shared development deployment

The local deployment serves this checkout but uses a separate runtime storage
path. Its `.env` sets `LARAVEL_STORAGE_PATH`, so bootstrapping Laravel from CLI
can write the live compiled-view cache too. Do not assume `web/storage` is active.

Laravel's atomic file replacement applies `0777 - umask()`. A CLI process with
umask `0077` creates compiled views with mode `0700`, masking inherited ACL
access for the `http` PHP-FPM worker. Repeated authenticated console requests
then fail with `include(...storage/framework/views/...php): Permission denied`.
A default directory ACL alone does not prevent recurrence.

Rendering tests must set `VIEW_COMPILED_PATH` to an isolated temporary directory
before bootstrapping Laravel and remove that directory afterwards. The Flux, console, owner-connection, profile-menu and sidebar-action
rendering checks do this. The sidebar-action check previously omitted this override
and recreated an unreadable live console view; its render now uses a temporary
cache removed in a `finally` block. For other ad hoc CLI render checks, use
an existing private scratch directory via that environment variable as well.
Never run live view-cache warming or clearing as the development CLI user.
Perform deliberate deployment cache operations as the PHP-FPM service identity.
Do not change the process-wide umask to solve this: it also governs secret files.

For recovery, inspect the active Laravel log and the exact failing generated
file with `namei -l` and `getfacl`. On this host the worker is `http`; restore only
its read access to affected generated PHP files (for example,
`setfacl -m u:http:r--,m::r-- <compiled-view.php>` as the file owner).
Do not recursively make storage, credentials, sessions or database files public.
Verify effective access and repeat the authenticated request. A successful public
login page does not verify the authenticated console.
