<section id="enrollment-panel" hidden class="space-y-3 rounded-lg border border-zinc-200 p-3 dark:border-zinc-700" aria-label="Create ChatGPT account">
    <flux:heading size="sm">Add ChatGPT account</flux:heading>
    <flux:text>Sign in privately on the provider website. Do not paste passwords, tokens or device codes into chat. API-key account setup is not available here yet.</flux:text>
    <label class="block text-sm">Provider connection<select id="enrollment-provider" class="block w-full rounded border p-2 dark:bg-zinc-800"></select></label>
    <flux:input id="enrollment-alias" label="Account alias" placeholder="personal-chatgpt" autocomplete="off" maxlength="64" />
    <flux:input id="enrollment-label" label="Display name" placeholder="Personal ChatGPT" autocomplete="off" maxlength="128" />
    <div id="enrollment-private" hidden class="space-y-2">
        <p>Open <a id="enrollment-link" target="_blank" rel="noopener noreferrer" class="underline">ChatGPT device sign-in</a> and enter this code:</p>
        <p id="enrollment-code" class="font-mono text-lg" aria-label="Private sign-in code"></p>
    </div>
    <p id="enrollment-status" role="status" class="text-sm"></p>
    <div class="flex flex-wrap gap-2">
        <flux:button id="enrollment-start" type="button" size="sm">Start sign-in</flux:button>
        <flux:button id="enrollment-check" type="button" size="sm">Check sign-in</flux:button>
        <flux:button id="enrollment-cancel" type="button" size="sm">Cancel sign-in</flux:button>
        <flux:button id="enrollment-close" type="button" size="sm" variant="ghost">Hide</flux:button>
    </div>
</section>
