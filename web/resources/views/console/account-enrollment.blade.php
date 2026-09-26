<section id="enrollment-panel" hidden class="space-y-4" aria-label="Add ChatGPT account" aria-busy="false">
    <div class="flex items-start justify-between gap-3">
        <div><flux:heading id="enrollment-title" size="sm">Connect ChatGPT</flux:heading><flux:text class="mt-1">Your sign-in stays on your Vessel.</flux:text></div>
        <flux:button id="enrollment-close" type="button" size="sm" variant="ghost" icon="x-mark" aria-label="Close account sign-in" />
    </div>
    <div id="enrollment-setup" class="space-y-3">
        <div id="enrollment-provider-field" hidden><flux:select id="enrollment-provider" label="ChatGPT connection" /></div>
        <flux:input id="enrollment-label" label="Account name" placeholder="e.g. Personal or Work" autocomplete="off" maxlength="128" description="A name to recognize this account in Helm." />
    </div>
    <div id="enrollment-private" hidden class="space-y-4">
        <flux:text>Enter this code on the ChatGPT sign-in page.</flux:text>
        <flux:text id="enrollment-code" class="rounded-lg border border-zinc-200 bg-white px-4 py-3 text-center font-mono text-2xl font-semibold tracking-widest select-all dark:border-zinc-600 dark:bg-zinc-900" aria-label="Private sign-in code" />
        <flux:button id="enrollment-link" href="https://auth.openai.com/codex/device" target="_blank" rel="noopener noreferrer" variant="primary" icon:trailing="arrow-top-right-on-square" class="w-full">Open ChatGPT sign-in</flux:button>
    </div>
    <flux:text id="enrollment-status" role="status" aria-live="polite" />
    <div class="flex flex-wrap items-center justify-end gap-2">
        <flux:button id="enrollment-cancel" type="button" size="sm" variant="ghost" hidden>Cancel sign-in</flux:button>
        <flux:button id="enrollment-check" type="button" size="sm" variant="primary" hidden>Check sign-in</flux:button>
        <flux:button id="enrollment-start" type="button" size="sm" variant="primary" disabled>Continue with ChatGPT</flux:button>
    </div>
</section>
