<?php

namespace App\Http\Controllers;

use App\Models\VesselConnection;
use App\Models\VesselPairing;
use Illuminate\Contracts\View\View;
use Illuminate\Http\Request;

class ReactConsoleController extends Controller
{
    public function redirect(Request $request): \Illuminate\Http\RedirectResponse
    {
        return redirect()->route('console', $request->boolean('manage-vessels') ? ['manage-vessels' => 1] : []);
    }

    public function __invoke(Request $request): View
    {
        return view('console.react', [
            'bootstrap' => [
                'connectionStatus' => $request->session()->get('status'),
                'connectionError' => $request->session()->has('errors') ? 'Connection not confirmed. Check pending pairings and supplied fields before trying again.' : null,
                'tenantId' => $request->user()->tenant_id,
                'principalId' => $request->user()->tenant->principal_id,
                'pairings' => VesselPairing::where('tenant_id', $request->user()->tenant_id)->where('status', 'pending')->get(['id', 'name'])->toArray(),
                'vessels' => VesselConnection::where('tenant_id', $request->user()->tenant_id)
                    ->get(['id', 'name', 'vessel_id'])->toArray(),
                'ticketUrl' => route('console.ticket', absolute: false),
                'connectionsUrl' => route('connections', absolute: false),
                'logoutUrl' => route('console.logout', absolute: false),
            ],
        ]);
    }
}
