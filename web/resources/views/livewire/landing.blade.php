<div>
    <x-site-header />

    <main id="main" tabindex="-1" class="mx-auto max-w-7xl px-6 lg:px-8">
        <section class="grid gap-12 py-12 sm:py-20 lg:grid-cols-2 lg:items-center">
            <div class="space-y-6">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Your machines are ready for more</flux:text>
                <flux:heading level="1" size="xl" class="text-4xl! leading-tight! tracking-tight sm:text-6xl!">Put every machine to work.</flux:heading>
                <flux:text class="max-w-xl text-lg leading-relaxed">Run coding agents across the computers you already own. Start more work, keep it moving when you disconnect, and steer it all from Helm.</flux:text>
                <div class="flex flex-wrap gap-3">
                    <flux:button href="{{ route('products.helm') }}" variant="primary" icon:trailing="arrow-right">Meet Helm</flux:button>
                    <flux:button href="#products" variant="ghost" icon:trailing="arrow-down">Explore the suite</flux:button>
                </div>
                <flux:text>Built for Linux first. Web and mobile are next.</flux:text>
            </div>

            <flux:card variant="soft" class="space-y-6 sm:p-8" role="complementary" aria-label="How the suite works">
                <flux:heading level="2" size="xl">One place to steer.</flux:heading>
                <flux:text>Helm connects to local and remote Vessels running independent Voyages.</flux:text>
                <flux:separator />
                <dl class="space-y-5">
                    <div class="space-y-1">
                        <dt><flux:heading>Helm</flux:heading></dt>
                        <dd><flux:text>See everything. Steer work from your command center.</flux:text></dd>
                    </div>
                    <div class="space-y-1">
                        <dt><flux:heading>Local and remote Vessels</flux:heading></dt>
                        <dd><flux:text>Your workstation and your server, with authority kept on each machine.</flux:text></dd>
                    </div>
                    <div class="space-y-1">
                        <dt><flux:heading>Independent Voyages</flux:heading></dt>
                        <dd><flux:text>Refactor an API, review changes, or prepare a release in separate sessions.</flux:text></dd>
                    </div>
                </dl>
            </flux:card>
        </section>

        <flux:separator />
        <ul class="grid gap-4 py-6 sm:grid-cols-2 lg:grid-cols-4">
            <li><flux:text>No artificial connection caps</flux:text></li>
            <li><flux:text>Work survives disconnects</flux:text></li>
            <li><flux:text>Credentials stay on your machines</flux:text></li>
            <li><flux:text>Local and remote execution</flux:text></li>
        </ul>
        <flux:separator />

        <section id="products" class="space-y-8 py-12 sm:py-16">
            <div class="max-w-2xl space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">The product suite</flux:text>
                <flux:heading level="2" size="xl">Three products. One job: ship more.</flux:heading>
                <flux:text class="text-base">Start with the result you want, then see what each product brings to the crew.</flux:text>
            </div>
            <div class="grid gap-6 md:grid-cols-3">
                <flux:card class="flex flex-col items-start gap-4" role="article">
                    <flux:text>01 / Command center</flux:text>
                    <flux:heading level="3" size="xl">Helm</flux:heading>
                    <flux:text class="grow">See every session and steer work across your machines.</flux:text>
                    <flux:button href="{{ route('products.helm') }}" icon:trailing="arrow-right">Meet Helm</flux:button>
                </flux:card>
                <flux:card class="flex flex-col items-start gap-4" role="article">
                    <flux:text>02 / Machine fleet</flux:text>
                    <flux:heading level="3" size="xl">Vessel</flux:heading>
                    <flux:text class="grow">Turn the computers you control into dependable agent capacity.</flux:text>
                    <flux:button href="{{ route('products.vessel') }}" icon:trailing="arrow-right">Meet Vessel</flux:button>
                </flux:card>
                <flux:card class="flex flex-col items-start gap-4" role="article">
                    <flux:text>03 / Persistent work</flux:text>
                    <flux:heading level="3" size="xl">Voyage</flux:heading>
                    <flux:text class="grow">Keep every agent session durable and ready to continue.</flux:text>
                    <flux:button href="{{ route('products.voyage') }}" icon:trailing="arrow-right">Meet Voyage</flux:button>
                </flux:card>
            </div>
        </section>

        <flux:separator />
        <section class="grid gap-6 py-12 sm:py-16 md:grid-cols-2">
            <div class="space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Coming into port</flux:text>
                <flux:heading level="2" size="xl">Your fleet is waiting.</flux:heading>
            </div>
            <div class="space-y-6">
                <flux:text class="text-base">Helm, Vessel, and Voyage are in active development from Foleybridge.Software.</flux:text>
                <flux:button href="{{ route('products.helm') }}" variant="primary" icon:trailing="arrow-right">See what ships first</flux:button>
            </div>
        </section>
    </main>

    <x-site-footer />
</div>
