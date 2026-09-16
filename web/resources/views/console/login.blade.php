<x-layouts.console>
    <main class="mx-auto flex min-h-dvh w-full max-w-md flex-col justify-center px-6 py-12">
        <flux:heading size="xl">Sign in to Helm</flux:heading>
        <flux:text class="mt-2">Your own workspace. Connect your Vessels and continue your voyages.</flux:text>
        @error('oauth') <flux:callout variant="danger" class="mt-6" role="alert">{{ $message }}</flux:callout> @enderror
        @forelse (\App\Http\Controllers\OAuthController::providers() as $provider => $label)
            <div class="my-6">
                @if ($provider === 'google')
                    <x-google-sign-in />
                @else
                    <flux:button href="{{ route('oauth.redirect', $provider) }}" variant="outline" class="w-full">Continue with {{ $label }}</flux:button>
                @endif
            </div>
        @empty
            <flux:callout class="my-6" role="status">Sign-in providers are being configured. Please check back shortly.</flux:callout>
        @endforelse
        <flux:text size="sm">Each provider identity has its own personal tenant. Accounts are not merged by email.</flux:text>
        <flux:link href="{{ route('home') }}" class="mt-4 inline-block">Back to landing</flux:link>
    </main>
</x-layouts.console>
