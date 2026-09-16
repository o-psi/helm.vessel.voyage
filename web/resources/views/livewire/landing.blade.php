<div>
    <x-site-header />

    <main id="main" tabindex="-1" class="mx-auto max-w-7xl px-6 lg:px-8">
        <section class="grid gap-12 py-12 sm:py-20 lg:grid-cols-2 lg:items-center">
            <div class="space-y-6">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Helm on the web and in your terminal</flux:text>
                <flux:heading level="1" size="xl" class="text-4xl! leading-tight! tracking-tight sm:text-6xl!">Your agents. Your machines. Your Helm.</flux:heading>
                <flux:text class="max-w-xl text-lg leading-relaxed">Your agents run on your machines. Steer them from your browser or terminal, follow their progress, and return to the same work after you disconnect.</flux:text>
                <div class="flex flex-wrap gap-3">
                    <flux:button href="{{ route('console') }}" variant="primary" icon:trailing="arrow-right">Open Helm Web</flux:button>
                    <flux:button href="#install" variant="ghost" icon:trailing="arrow-down">Set up your Linux machine</flux:button>
                </div>
                <flux:text>Already running a Vessel? Sign in and connect it. New here? Install Vessel and Voyage on a Linux machine first. Helm Web is an interface, not hosted agent compute.</flux:text>
            </div>

            <flux:card variant="soft" class="space-y-6 sm:p-8" role="complementary" aria-label="How the suite works">
                <flux:heading level="2" size="xl">One place to steer.</flux:heading>
                <flux:text>Vessel and Voyage run the work on your computer or server. Helm gives you a choice of interfaces.</flux:text>
                <flux:separator />
                <dl class="space-y-5">
                    <div class="space-y-1">
                        <dt><flux:heading>Helm</flux:heading></dt>
                        <dd><flux:text>Open Helm in your browser or use the Linux terminal app.</flux:text></dd>
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
            <li><flux:text>Local and remote Vessels</flux:text></li>
            <li><flux:text>Work survives disconnects</flux:text></li>
            <li><flux:text>Provider credentials stay on your machines</flux:text></li>
            <li><flux:text>Separate, independent sessions</flux:text></li>
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
                    <flux:text class="grow">Start and steer voyages from Helm Web or the Linux terminal app.</flux:text>
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
                    <flux:text class="grow">Run each agent session in its own process, with saved conversation history.</flux:text>
                    <flux:button href="{{ route('products.voyage') }}" icon:trailing="arrow-right">Meet Voyage</flux:button>
                </flux:card>
            </div>
        </section>

        <flux:separator />
        <section id="install" aria-labelledby="install-heading" class="space-y-8 py-12 sm:py-16">
            <div class="max-w-2xl space-y-4">
                <flux:text class="font-medium text-emerald-700! dark:text-emerald-400!">Ready to download</flux:text>
                <flux:heading id="install-heading" level="2" size="xl">Set up the machine that does the work.</flux:heading>
                <flux:text class="text-base">Install Vessel and Voyage together on a workstation or server you control. The v1.0.0 Linux download includes both, plus the optional Helm terminal app and installer. Download the archive and checksum into the same folder, then verify before extracting.</flux:text>
            </div>
            <div class="grid gap-6 lg:grid-cols-2">
                <flux:card class="min-w-0 space-y-5">
                    <flux:heading level="3" size="lg">1. Download and verify</flux:heading>
                    <div class="flex flex-wrap gap-3">
                        <flux:button href="https://github.com/o-psi/helm.vessel.voyage/releases/download/v1.0.0/voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz" variant="primary" icon:trailing="arrow-down-tray">Linux x86-64 archive</flux:button>
                        <flux:button href="https://github.com/o-psi/helm.vessel.voyage/releases/download/v1.0.0/voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz.sha256">SHA-256 checksum</flux:button>
                    </div>
                    <pre tabindex="0" aria-label="Verify and extract the Linux release" class="overflow-x-auto rounded-lg bg-zinc-100 p-4 text-sm text-zinc-800 dark:bg-zinc-900 dark:text-zinc-200"><code>sha256sum -c voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz</code></pre>
                    <flux:text>Continue only if the checksum reports OK. Checksums verify file integrity; this release does not include independent publisher signatures.</flux:text>
                </flux:card>
                <flux:card class="min-w-0 space-y-5">
                    <flux:heading level="3" size="lg">2. Review and install</flux:heading>
                    <pre tabindex="0" aria-label="Open the release installer" class="overflow-x-auto rounded-lg bg-zinc-100 p-4 text-sm text-zinc-800 dark:bg-zinc-900 dark:text-zinc-200"><code>cd voyage-v1.0.0-x86_64-unknown-linux-gnu
./bin/voyage-installer --bin-dir "$PWD/bin"</code></pre>
                    <flux:text>Run as your ordinary user, not with sudo. The installer lets you review installation and service choices before applying them. Managed installation needs Python 3.11+ and a running systemd user manager.</flux:text>
                    <flux:text>Add <code>$HOME/.local/bin</code> to your PATH. For terminal use, open <code>helm</code>. For web use, continue with the connection setup below; you do not need to run the Helm terminal app.</flux:text>
                    <flux:button href="https://github.com/o-psi/helm.vessel.voyage/blob/v1.0.0/docs/getting-started.md" variant="ghost" icon:trailing="arrow-right">Terminal first-task guide</flux:button>
                </flux:card>
            </div>
            <flux:card variant="soft" class="space-y-5">
                <flux:heading level="3" size="lg">3. Connect your machine to Helm Web</flux:heading>
                <flux:text>Helm Web needs a publicly reachable HTTPS/WSS endpoint for your Vessel, with authentication and TLS configured. A local-only installation is not enough. Sign in to Helm Web, then pair that Vessel with your account.</flux:text>
                <flux:text>Configure a provider account on the machine running Vessel and Voyage. Provider credentials and execution stay there; the web service holds the connection credential needed to reach your Vessel. Provider access and billing are separate, and no model credits are included.</flux:text>
                <div class="flex flex-wrap gap-3">
                    <flux:button href="https://github.com/o-psi/helm.vessel.voyage/blob/main/docs/helm-web.md#connect-your-vessels" icon:trailing="arrow-right">Web connection setup</flux:button>
                    <flux:button href="{{ route('console') }}" variant="primary">Open Helm Web</flux:button>
                </div>
            </flux:card>
            <div class="flex flex-wrap gap-3">
                <flux:button href="https://github.com/o-psi/helm.vessel.voyage/releases/tag/v1.0.0" icon:trailing="arrow-right">Release notes and all downloads</flux:button>
                <flux:button href="#products" variant="ghost">Explore the suite</flux:button>
            </div>
            <flux:text>This binary release supports Linux x86-64 with glibc 2.39 or newer, not ARM64, Alpine/musl, macOS, or Windows. The web console is separate from the download. See the release notes for verification details and known limitations.</flux:text>
        </section>
    </main>

    <x-site-footer />
</div>
