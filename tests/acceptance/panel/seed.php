<?php

declare(strict_types=1);

use App\Enums\EdgeRuntimeStatus;
use App\Enums\EdgeStatus;
use App\Enums\PricingMeter;
use App\Enums\ProviderModelCatalogVisibility;
use App\Enums\ProviderModelInvocationScope;
use App\Enums\UserStatus;
use App\Models\Edge;
use App\Models\ModelPrice;
use App\Models\ProviderModel;
use App\Models\TlsCertificate;
use App\Models\User;
use App\Services\CertificateMaterialInspector;
use App\Services\Console\NodeAccessData;
use App\Services\Orbit\CodexCatalogBinding;
use App\Services\Orbit\OrbitAccountData;
use Database\Seeders\FoundationSeeder;
use Illuminate\Contracts\Console\Kernel;
use Illuminate\Database\Connection;
use Illuminate\Support\Facades\DB;

// Fixture for the separately pinned Panel 01d8f67a5af6ba189c933a6f214de5ecfb8010e3.
// It never migrates, resets, grants an installation, issues a key, or makes a node online.
ini_set('display_errors', '0');
ini_set('log_errors', '0');
ob_start();

function rejectSeed(string $code): never
{
    throw new RuntimeException($code);
}

/** @return array{panelRoot: string, database: string, email: string, password: string} */
function seedInput(): array
{
    $json = stream_get_contents(STDIN, 16_385);
    if (! is_string($json) || strlen($json) > 16_384) {
        rejectSeed('invalid_input');
    }
    $input = json_decode($json, true, 8, JSON_THROW_ON_ERROR);
    $keys = is_array($input) ? array_keys($input) : [];
    sort($keys);
    if ($keys !== ['database', 'email', 'panelRoot', 'password']
        || ! is_string($input['panelRoot']) || ! str_starts_with($input['panelRoot'], '/')
        || ! is_string($input['database'])
        || preg_match('/\Aorivelle_orbit_acceptance_[a-z0-9_]{8,38}\z/', $input['database']) !== 1
        || ! is_string($input['email'])
        || preg_match('/\A[a-z0-9._+-]{1,100}@example\.test\z/', $input['email']) !== 1
        || ! is_string($input['password'])
        || preg_match('/\A[\x21-\x7e]{24,256}\z/', $input['password']) !== 1) {
        rejectSeed('invalid_input');
    }

    return $input;
}

/** Only the fixed migration's inert rows may predate this fixture. */
function assertFreshDatabase(Connection $db, string $database): void
{
    if ($db->getDriverName() !== 'mysql' || $db->getTablePrefix() !== ''
        || $db->getDatabaseName() !== $database
        || ($db->selectOne('SELECT DATABASE() AS name')->name ?? null) !== $database) {
        rejectSeed('database_scope_invalid');
    }
    $tables = $db->getSchemaBuilder()->getTableListing($database, false);
    foreach (['users', 'provider_models', 'model_prices', 'edges', 'tls_certificates', 'api_keys',
        'orbit_installations', 'orbit_authorization_requests', 'orbit_sessions',
        'orbit_session_tokens', 'orbit_installation_keys', 'upstream_accounts', 'ledger_entries', 'platform_settings'] as $required) {
        if (! in_array($required, $tables, true)) {
            rejectSeed('database_schema_invalid');
        }
    }
    foreach ($tables as $table) {
        if (in_array($table, ['migrations', 'cache', 'cache_locks'], true)) {
            continue;
        }
        if ($table === 'platform_settings') {
            // UserAiCustodySchema::install() persists this disabled switch during the core migration.
            $rows = $db->table($table)->get();
            $row = $rows->first();
            if ($rows->count() !== 1 || $row->key !== 'user_ai_custody_enabled'
                || $row->value !== 'false' || $row->type !== 'boolean' || $row->group !== 'general'
                || (bool) $row->is_secret || $row->description !== '本人私人 AI 账户托管'
                || $row->updated_by !== null || $row->original_updated_by !== null
                || $row->retired_updater_membership_id !== null) {
                rejectSeed('database_not_fresh');
            }
            continue;
        }
        if ($table === 'provider_models') {
            $baseline = ['codex-auto-review', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.5',
                'gpt-5.6-luna', 'gpt-5.6-sol', 'gpt-5.6-terra'];
            $rows = $db->table($table)->get();
            if ($rows->count() !== count($baseline)) {
                rejectSeed('database_not_fresh');
            }
            foreach ($rows as $row) {
                if (! in_array($row->slug, $baseline, true) || $row->provider_id !== 'openai'
                    || $row->slug !== $row->upstream_model || (bool) $row->enabled || (bool) $row->public_enabled
                    || $row->codex_catalog_binding !== null || $row->allowed_reasoning_efforts !== null
                    || $row->supported_reasoning_efforts !== null
                    || $row->catalog_visibility !== ($row->slug === 'codex-auto-review' ? 'hidden' : 'listed')
                    || $row->invocation_scope !== ($row->slug === 'codex-auto-review' ? 'codex_auto_review' : 'public')) {
                    rejectSeed('database_not_fresh');
                }
            }
            continue;
        }
        if ($table === 'upstream_account_rpm_locks' || $table === 'admission_queue_scopes') {
            $key = $table === 'upstream_account_rpm_locks' ? 'upstream_account_id' : 'scope_key';
            $value = $table === 'upstream_account_rpm_locks'
                ? '00000000000000000000000000' : hash('sha256', 'admission-queue-scope-row-creation');
            if ($db->table($table)->count() !== 1 || ! $db->table($table)->where($key, $value)->exists()) {
                rejectSeed('database_not_fresh');
            }
            continue;
        }
        if ($db->table($table)->exists()) {
            rejectSeed('database_not_fresh');
        }
    }
}

try {
    $input = seedInput();
    $panelRoot = realpath($input['panelRoot']);
    if ($panelRoot === false || ! is_file($panelRoot.'/vendor/autoload.php')
        || ! is_file($panelRoot.'/bootstrap/app.php')) {
        rejectSeed('panel_bootstrap_unavailable');
    }
    require $panelRoot.'/vendor/autoload.php';
    $app = require $panelRoot.'/bootstrap/app.php';
    $app->make(Kernel::class)->bootstrap();
    if ($app->environment() !== 'local') {
        rejectSeed('environment_not_local');
    }
    $db = DB::connection();
    assertFreshDatabase($db, $input['database']);

    $result = $db->transaction(function () use ($app, $db, $input): array {
        assertFreshDatabase($db, $input['database']);
        $foundation = $app->make(FoundationSeeder::class);
        $foundation->setContainer($app)->run();

        $user = User::query()->create([
            'name' => 'Synthetic Orbit acceptance user', 'email' => $input['email'],
            'password' => $input['password'], 'status' => UserStatus::Active, 'is_staff' => false,
        ]);
        // A starting fixture balance is not a recharge, usage charge, or ledger event.
        $user->forceFill(['email_verified_at' => now(), 'balance' => '10.00000000'])->save();
        $user->assignRole('User');

        foreach (['orbit-synthetic-text' => true, 'orbit-synthetic-denied' => false] as $slug => $public) {
            $model = new ProviderModel([
                'provider_id' => 'openai', 'slug' => $slug,
                'upstream_model' => $public ? 'gpt-6.1-sol' : 'gpt-6-luna',
                'display_name' => $public ? 'Synthetic public text model' : 'Synthetic non-public text model',
                'model_category' => 'text', 'enabled' => true, 'public_enabled' => $public,
                'catalog_visibility' => ProviderModelCatalogVisibility::Listed,
                'invocation_scope' => ProviderModelInvocationScope::Public, 'available' => true,
                'capabilities' => ['openai.responses', 'anthropic.messages', 'streaming.sse'],
                'supported_reasoning_efforts' => ['low', 'medium', 'high'],
                'allowed_reasoning_efforts' => ['medium'],
            ]);
            if ($public) {
                $model->codex_catalog_binding = $app->make(CodexCatalogBinding::class)->normalize($model, [
                    'profile_id' => CodexCatalogBinding::PROFILE_ID, 'template_slug' => 'gpt-6.1-sol',
                ]);
            }
            $model->save();
            foreach ([PricingMeter::Input, PricingMeter::Output] as $meter) {
                ModelPrice::query()->create([
                    'provider_model_id' => $model->id, 'meter' => $meter, 'unit_size' => 1_000,
                    'unit_price' => '1.000000000000', 'original_unit_price' => '1.000000000000',
                ]);
            }
        }

        // Required node FK material only: generated in memory, never served or trusted as ingress TLS.
        $domain = 'synthetic-node.orbit-e2e.test';
        $key = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_RSA, 'private_key_bits' => 2048]);
        $csr = $key === false ? false : openssl_csr_new(['commonName' => $domain], $key, ['digest_alg' => 'sha256']);
        $certificate = $csr === false ? false : openssl_csr_sign($csr, null, $key, 2, ['digest_alg' => 'sha256']);
        if ($certificate === false || ! openssl_x509_export($certificate, $certificatePem)
            || ! openssl_pkey_export($key, $privateKeyPem)) {
            rejectSeed('synthetic_material_failed');
        }
        $tls = TlsCertificate::query()->create([
            'name' => 'Synthetic unserved offline-node certificate',
            ...$app->make(CertificateMaterialInspector::class)->inspect($certificatePem, $privateKeyPem),
            'certificate_pem' => $certificatePem, 'private_key_pem' => $privateKeyPem,
        ]);
        unset($certificatePem, $privateKeyPem, $key, $csr, $certificate);
        $node = Edge::query()->create([
            'name' => 'Synthetic offline catalog node', 'domain' => $domain, 'listen_port' => 443,
            'tls_certificate_id' => $tls->id, 'trusted_source_ips' => [], 'country_code' => 'US',
            'status' => EdgeStatus::Active, 'runtime_status' => EdgeRuntimeStatus::Offline,
        ]);

        $catalog = $app->make(OrbitAccountData::class)->modelCatalog($user);
        $nodes = $app->make(NodeAccessData::class)->runtime($user)['nodes'];
        if (array_column($catalog, 'id') !== ['orbit-synthetic-text']
            || ($catalog[0]['codex_catalog_binding']['state'] ?? null) !== 'declared'
            || ($catalog[0]['reasoning']['default_effort'] ?? null) !== 'medium'
            || count($nodes) !== 1 || $nodes[0]['id'] !== $node->id || $nodes[0]['service_state'] !== 'unavailable') {
            rejectSeed('fixture_projection_invalid');
        }
        foreach (['api_keys', 'orbit_installations', 'orbit_authorization_requests', 'orbit_sessions',
            'orbit_session_tokens', 'orbit_installation_keys', 'upstream_accounts', 'edge_credentials',
            'edge_heartbeats', 'dispatches', 'ledger_entries', 'recharge_orders'] as $table) {
            if ($db->table($table)->exists()) {
                rejectSeed('fixture_boundary_invalid');
            }
        }

        return ['ok' => true, 'code' => 'seeded', 'accountId' => (string) $user->id,
            'nodeId' => $node->id, 'modelIds' => ['orbit-synthetic-text'],
            'deniedModelIds' => ['orbit-synthetic-denied'], 'databaseVerified' => true,
            'nodeAvailable' => false, 'nodeState' => 'unavailable'];
    });
} catch (Throwable $error) {
    $codes = ['invalid_input', 'database_scope_invalid', 'database_schema_invalid', 'database_not_fresh',
        'panel_bootstrap_unavailable', 'environment_not_local', 'synthetic_material_failed',
        'fixture_projection_invalid', 'fixture_boundary_invalid'];
    $result = ['ok' => false, 'code' => in_array($error->getMessage(), $codes, true) ? $error->getMessage() : 'seed_failed'];
}

while (ob_get_level() > 0) {
    ob_end_clean();
}
fwrite(STDOUT, json_encode($result, JSON_THROW_ON_ERROR).PHP_EOL);
exit($result['ok'] ? 0 : 1);
