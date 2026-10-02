# Participant fixture account isolation

Prepared repair for the coordinated [v1.0.3 release gate](https://github.com/o-psi/helm.vessel.voyage/issues/375).
The four maintained participant tests failed at fixture construction in the
`9539be5` gate, before assignment/cancellation/budget assertions, with the closed
`invalid private account registry` diagnostic.

## Cause and prepared boundary

The fixture passed its temporary Vessel root to production `device_service`.
For ordinary Vessel roots that function deliberately selects
`Registry::default_host()`, using the caller's HOME/XDG account namespace, and
then initializes/observes the ChatGPT connection. A temporary Vessel directory
therefore did not isolate these tests from the real user account store. The
recorded error originates from deserializing that external registry. This audit
has not read its contents or credentials, and does not attribute its state to a
particular user action or establish a new production account-store defect.

The test-only factory now selects `Registry::new(fixture_root/accounts)` and
initializes its real connection/schema/private storage boundary. It constructs
the actual `DeviceService` with the unchanged production
`enrollment_authorized` current-grant callback. It does not replace authorization
with an always-allowed mock or start an enrollment driver. Fixture assertions
require its owned registry file, exactly one connection and no accounts.

No global HOME/XDG mutation, environment lock/race, inherited credential, provider
request, account migration or native service effect is used. The factory stays in
the standard test source module and is reexported only under `cfg(test)`; normal
production `device_service` selection and storage validation remain unchanged.
All temporary state is inside each existing uniquely owned fixture and covered
by its existing cleanup.

## Preserved acceptance

The original four test bodies remain unchanged:

- Exact retry observes uncertain reservation without redispatch and refuses a
  changed assignment or grant.
- Cancellation survives polling, reopened persisted state and registration
  failure, without claiming cleanup.
- Legacy revoked grants recover cancellation and a terminal tombstone stays
  terminal.
- Goal budget tokens and parent/session/command identities belong to the exact
  persisted assignment and changed envelopes are refused.

Formatting and diff checks are source checks only. Root owns independent review,
the next coordinated workspace verification/coverage/current-object accounting,
publication and hosted follow-through. No test/build/coverage was run during
preparation, and no existing private account/environment content was inspected.
