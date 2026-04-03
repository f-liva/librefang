---
status: diagnosed
trigger: "TokenRotationDriver non prova tutti i profili Claude Code"
created: 2026-04-03T22:00:00Z
updated: 2026-04-03T22:30:00Z
---

## Current Focus

hypothesis: resolve_driver() bypassa il TokenRotationDriver creando un singolo ClaudeCodeDriver via driver_cache
test: traccia del flusso kernel -> resolve_driver -> driver_cache -> create_driver
expecting: conferma che il driver usato dall'agent loop non e' il TokenRotationDriver
next_action: diagnosi completata, report scritto

## Symptoms

expected: Quando tutti e 3 i profili Claude Code sono rate-limited, i log dovrebbero mostrare 3 tentativi (uno per profilo) con "Claude Code CLI streaming subprocess exited with error"
actual: I log mostrano solo 1 "Claude Code CLI exited with error" per tentativo, poi l'errore viene propagato immediatamente
errors: "You're out of extra usage - resets 10am (UTC)" con un solo exit_code=1
reproduction: Succede ogni volta che tutti i profili sono esauriti — il daemon prova solo 1 profilo invece di 3
started: Probabilmente da sempre — il TokenRotationDriver non e' mai stato usato nel percorso streaming/message

## Eliminated

- hypothesis: Tutti i profili sono in cooldown da un tentativo precedente
  evidence: Il problema e' strutturale — resolve_driver() non ritorna MAI il TokenRotationDriver. Il cooldown e' irrilevante.
  timestamp: 2026-04-03T22:15:00Z

- hypothesis: Il streaming path bypassa il TokenRotationDriver
  evidence: Parzialmente corretto ma la causa e' piu' profonda — non e' specifico dello streaming. ANCHE il path non-streaming ha lo stesso bug. resolve_driver() bypassa il TokenRotationDriver per QUALSIASI chiamata.
  timestamp: 2026-04-03T22:20:00Z

## Evidence

- timestamp: 2026-04-03T22:05:00Z
  checked: kernel.rs riga 1695-1730 — costruzione del TokenRotationDriver al boot
  found: Il TokenRotationDriver viene creato con i 3 profili e salvato in driver_chain, che diventa self.default_driver
  implication: Il TokenRotationDriver esiste ed e' configurato correttamente al boot

- timestamp: 2026-04-03T22:08:00Z
  checked: kernel.rs riga 3463 — risoluzione del driver per streaming
  found: `let driver = self.resolve_driver(&entry.manifest)?;` — il driver usato dall'agent loop viene da resolve_driver(), NON da self.default_driver
  implication: Il driver passato all'agent loop potrebbe non essere il TokenRotationDriver

- timestamp: 2026-04-03T22:10:00Z
  checked: kernel.rs riga 7941-8035 — corpo di resolve_driver()
  found: resolve_driver() chiama self.driver_cache.get_or_create(&driver_config) che chiama create_driver(). Per claude-code, create_driver() crea un SINGOLO ClaudeCodeDriver senza config_dir e senza profili. Questa chiamata ha SEMPRE successo (key_required=false). Il fallback a self.default_driver (riga 8027) viene raggiunto SOLO se get_or_create fallisce — cosa che per claude-code non succede mai.
  implication: ROOT CAUSE TROVATA — resolve_driver() crea un ClaudeCodeDriver singolo "vanilla" che usa il profilo di default del sistema, ignorando completamente il TokenRotationDriver con i 3 profili configurati

- timestamp: 2026-04-03T22:12:00Z
  checked: drivers/mod.rs riga 398-405 — ProviderEntry per claude-code
  found: key_required=false, api_key_env="" — create_driver ha sempre successo per claude-code
  implication: Conferma che il branch di fallback a self.default_driver non viene mai preso

- timestamp: 2026-04-03T22:14:00Z
  checked: drivers/mod.rs riga 678-682 — create_driver per ApiFormat::ClaudeCode
  found: Crea ClaudeCodeDriver::with_timeout(base_url, skip_permissions, timeout) SENZA .with_config_dir() — il driver risultante usa il profilo di default del sistema (~/.claude), non i profili configurati in config.toml
  implication: Anche se il TokenRotationDriver venisse usato, i singoli driver dentro di esso hanno ciascuno un config_dir diverso. Il driver creato da create_driver usa il default.

- timestamp: 2026-04-03T22:18:00Z
  checked: kernel.rs riga 3716-3721 — passaggio del driver all'agent loop
  found: `run_agent_loop_streaming(..., driver, ...)` usa il driver da resolve_driver(), confermato che il TokenRotationDriver non viene mai usato
  implication: Conferma end-to-end del flusso

## Resolution

root_cause: |
  `resolve_driver()` (kernel.rs:7941) bypassa completamente il `TokenRotationDriver` configurato al boot.

  **Il meccanismo del bug:**

  1. Al boot (kernel.rs:1695-1726): il kernel crea un `TokenRotationDriver` con 3 `ClaudeCodeDriver`, ciascuno con un `config_dir` diverso (profilo 1, 2, 3). Questo viene salvato in `self.default_driver`.

  2. Per ogni messaggio (kernel.rs:3463): `resolve_driver()` viene chiamato. Questo metodo:
     - Crea un `DriverConfig` con `provider: "claude-code"` e `api_key: None`
     - Chiama `self.driver_cache.get_or_create(&driver_config)`
     - `create_driver()` crea un SINGOLO `ClaudeCodeDriver` senza `config_dir` (usa il profilo di default del sistema)
     - Questa creazione ha SEMPRE successo perche' `key_required=false` per claude-code
     - Il fallback a `self.default_driver` (che contiene il `TokenRotationDriver`) non viene MAI raggiunto

  3. Risultato: l'agent loop riceve un `ClaudeCodeDriver` singolo "vanilla" che usa solo il profilo di default, ignorando completamente la rotazione tra i 3 profili.

  **Perche' i log mostrano 1 solo tentativo:** perche' c'e' effettivamente 1 solo driver (non wrappato in TokenRotationDriver), quindi 1 sola chiamata CLI, 1 solo errore.

fix: |
  **Proposta di fix (NON implementata):**

  Il fix deve far si' che `resolve_driver()` usi il `TokenRotationDriver` quando il provider e' claude-code e ci sono profili configurati. Due approcci possibili:

  **Approccio A (minimale, consigliato):** In `resolve_driver()`, quando il provider dell'agente corrisponde al default provider E il `self.default_driver` e' un `TokenRotationDriver` (o piu' semplicemente, quando ci sono profili configurati), ritornare direttamente `Arc::clone(&self.default_driver)` PRIMA di tentare `driver_cache.get_or_create()`.

  ```rust
  // In resolve_driver(), dopo aver determinato agent_provider e default_provider:
  // Se il provider e' lo stesso del default E ci sono profili CLI configurati,
  // usa direttamente il default_driver (che contiene il TokenRotationDriver)
  if agent_provider == default_provider
      && !has_custom_key
      && !has_custom_url
      && !cfg.default_model.profiles.is_empty()
  {
      return Ok(Arc::clone(&self.default_driver));
  }
  ```

  **Approccio B (piu' strutturale):** Modificare `DriverCache` per supportare il caching di `TokenRotationDriver`, oppure memorizzare il `TokenRotationDriver` nel cache con una chiave speciale. Piu' complesso e non necessario.

  **Approccio A e' sufficiente** perche':
  - Il `TokenRotationDriver` e' gia' costruito al boot con i profili corretti
  - Non c'e' motivo di ricreare i driver per ogni messaggio — i profili non cambiano a runtime
  - Il fallback (agent con custom key/url su altro provider) continua a funzionare normalmente

verification: Non implementato — solo diagnosi
files_changed: []
