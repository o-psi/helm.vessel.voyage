// The console owns each action's availability independently. Do not disable the
// Flux composer host: its durable action attributes race with those controls.
// Read-only keeps the task selectable and leaves location/settings independent.
export function setComposerDisabled(composer, disabled) {
    const input = composer.querySelector('textarea');
    if (input) input.readOnly = Boolean(disabled);
}
