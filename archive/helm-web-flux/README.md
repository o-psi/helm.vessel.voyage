# Archived Livewire / Flux console

The operator promoted React to production in #336. This directory preserves the
retired console as historical source; it is not a runnable alternative, a route,
a production Vite entry, a Composer autoload root or part of the active test suite.
The former files retain their original paths relative to `web/` here. Historical
documentation describes the old implementation and its original relative links.
For a runnable historical checkout, use commit `acc3d4be41d1f0b2ed71fcdcf4325e558ec985e3`.

Production console work belongs in `web/resources/react`; shared transport,
account, updater and browser helpers remain in `web/resources/js`. Laravel login,
OAuth, tenant connections and public pages remain active, including their Flux
components. Do not restore the retired console to fix a production React issue.

The console-specific presentation tests are archived with their implementation.
Shared authority/transport tests remain active, and updater tests now exercise the
React-rendered controls with the same exact-operation controller.
