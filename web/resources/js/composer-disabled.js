// Flux owns action-button disabled attributes while the composer is disabled.
// Repeating a disabled mutation makes Flux replace its release callbacks with an
// empty set, permanently retaining those attributes. Only write transitions.
export function setComposerDisabled(composer, disabled) {
    if (composer.hasAttribute('disabled') !== Boolean(disabled)) {
        composer.toggleAttribute('disabled', Boolean(disabled));
    }
}
