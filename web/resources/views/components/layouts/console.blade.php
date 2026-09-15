<!doctype html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="csrf-token" content="{{ csrf_token() }}">
    <meta name="robots" content="noindex, nofollow">
    <title>Helm Console</title>
    @vite(['resources/css/console.css', 'resources/js/console.js'])
    @livewireStyles
    @fluxAppearance
</head>
<body class="helm-console">
    {{ $slot }}
    @fluxScripts
</body>
</html>
