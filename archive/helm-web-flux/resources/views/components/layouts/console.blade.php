<!doctype html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="csrf-token" content="{{ csrf_token() }}">
    <meta name="robots" content="noindex, nofollow">
    <title>Helm Console</title>
    @vite(['resources/css/console.css', 'resources/js/app.js', 'resources/js/console.js'])
    @livewireStyles
    @fluxAppearance
</head>
<body class="min-h-dvh bg-white font-sans text-zinc-800 antialiased dark:bg-zinc-800 dark:text-zinc-100">
    {{ $slot }}
    @fluxScripts
</body>
</html>
