<x-layouts.console>
    <main class="login-panel">
        <h1>Sign in to Helm</h1>
        <p>Your own workspace. Connect your Vessels and continue your voyages.</p>
        @error('oauth') <p role="alert">{{ $message }}</p> @enderror
        @forelse (\App\Http\Controllers\OAuthController::providers() as $provider => $label)
            <p><flux:button href="{{ route('oauth.redirect', $provider) }}" variant="primary">Continue with {{ $label }}</flux:button></p>
        @empty
            <p role="status">Sign-in providers are being configured. Please check back shortly.</p>
        @endforelse
        <p>Each provider identity has its own personal tenant. Accounts are not merged by email.</p>
        <a href="{{ route('home') }}">Back to landing</a>
    </main>
</x-layouts.console>
