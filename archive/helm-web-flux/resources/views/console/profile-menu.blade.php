<flux:dropdown x-data position="top" align="end">
    <flux:profile avatar="" :initials="mb_substr(auth()->user()?->name ?? 'U', 0, 1)" :chevron="false" avatar:size="xs" aria-label="Profile menu" />
    <flux:popover class="w-60 max-w-[calc(100vw-2rem)] min-w-0! space-y-3">
        <div class="min-w-0">
            <flux:heading class="truncate">{{ auth()->user()?->name }}</flux:heading>
            <flux:text size="sm" class="truncate">{{ auth()->user()?->email }}</flux:text>
        </div>
        <flux:separator />
        <flux:radio.group x-model="$flux.appearance" label="Appearance" variant="segmented" size="sm">
            <flux:radio value="light" icon="sun" class="min-w-0" aria-label="Light" title="Light" />
            <flux:radio value="dark" icon="moon" class="min-w-0" aria-label="Dark" title="Dark" />
            <flux:radio value="system" icon="computer-desktop" class="min-w-0" aria-label="System" title="System" />
        </flux:radio.group>
        <flux:button href="{{ route('connections') }}" size="sm" variant="ghost" icon="server-stack" class="w-full justify-start">Vessel connections</flux:button>
        <form method="post" action="{{ route('console.logout') }}">
            @csrf
            <flux:button type="submit" size="sm" variant="ghost" icon="arrow-right-start-on-rectangle" class="w-full justify-start">Sign out</flux:button>
        </form>
    </flux:popover>
</flux:dropdown>
