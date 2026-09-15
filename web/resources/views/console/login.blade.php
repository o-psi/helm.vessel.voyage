<x-layouts.console>
    <main class="login-panel">
        <h1>Helm</h1><p>Sign in to your private voyage console.</p>
        @error('login') <p role="alert">{{ $message }}</p> @enderror
        <form method="post" action="{{ route('console.login') }}">
            @csrf
            <flux:input name="password" type="password" label="Operator password" autocomplete="current-password" required />
            <flux:button type="submit" variant="primary">Sign in</flux:button>
        </form>
        <p>Credentials belong in this form, never in a conversation.</p>
    </main>
</x-layouts.console>
