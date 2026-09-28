# OurStick — CLAUDE.md

Οδηγίες για το Claude Code. Διάβασε ολόκληρο το αρχείο πριν γράψεις κώδικα.

## Τι είναι το OurStick

Ένα «εικονικό USB stick». Ο χρήστης ρίχνει φακέλους σε ένα stick, στέλνει link ή QR,
και ο παραλήπτης βλέπει και κατεβάζει τα αρχεία σε PC ή κινητό. Peer-to-peer,
κρυπτογραφημένο στη συσκευή, χωρίς λογαριασμούς, χωρίς όρια μεγέθους. Apache-2.0.

Tagline: "A USB stick you can send."

## Αρχιτεκτονική

```
crates/ourstick-core   ΟΛΗ η λογική. Καμία εξάρτηση από UI, Tauri, ή CLI.
crates/ourstick-cli    Λεπτό περίβλημα: share / get / list / seed.
apps/app               Tauri 2 (Rust commands + TypeScript UI). Desktop ΚΑΙ mobile.
apps/website           Static site για ourstick.com.
docs/                  architecture.md, protocol.md, decisions/ (ADRs).
```

Κανόνες που δεν παραβιάζονται:

1. **Το `ourstick-core` δεν κάνει import τίποτα από `apps/` ή `ourstick-cli`.**
   Αν χρειάζεται κάτι από το UI, ο σχεδιασμός είναι λάθος.
2. **Κάθε εξωτερική εξάρτηση μπαίνει πίσω από trait** στο core:
   `Transport` (iroh), `Store` (SQLite), `Cipher` (κρυπτογράφηση), `Clock`.
   Έτσι τα tests τρέχουν με in-memory υλοποιήσεις, χωρίς δίκτυο ή δίσκο.
3. **Η κατάσταση ζει σε SQLite** (`rusqlite`), με migrations σε `crates/ourstick-core/migrations/`.
   Όχι JSON αρχεία για state. Όχι global mutable state.
4. **Η κρυπτογράφηση σχεδιάζεται από την πρώτη μέρα**, ακόμα κι αν η πρώτη υλοποίηση
   είναι `NoCipher`. Το blob format έχει πάντα πεδίο `cipher: CipherId`.
   Το hash (BLAKE3) υπολογίζεται πάνω στο ciphertext, όχι στο plaintext.
   Το manifest κρυπτογραφείται όπως τα blobs.
5. **Κάθε public συνάρτηση έχει test.** Integration test στο
   `crates/ourstick-core/tests/` που σηκώνει δύο peers στην ίδια μηχανή και
   μεταφέρει έναν φάκελο. Αυτό το test είναι το πιο σημαντικό αρχείο του repo.
6. **Errors με `thiserror` στο core, `anyhow` μόνο σε CLI/app.**
7. **Async με tokio.** Καμία blocking I/O σε async context· χρησιμοποίησε `spawn_blocking`.
8. **Logging με `tracing`**, όχι `println!` εκτός από το CLI output.
9. **Κλειδιά (NodeId, StickId) δεν ξαναδημιουργούνται ποτέ αφού μοιραστεί ticket.**
   Αν χαθούν ή αλλάξουν, κάθε ticket που έχει μοιραστεί πεθαίνει.

## Έννοιες (domain model)

- **Stick**: όνομα + κλειδί + λίστα root φακέλων + δικαιώματα. Έχει `StickId` (public key).
- **StickId**: Ed25519 verifying key, χωριστό από το NodeId του iroh. Το signing key μένει μόνο
  στον owner. Οι ενημερώσεις manifest είναι υπογεγραμμένες εγγραφές (StickId, seq, manifest_hash).
- **Manifest**: το ευρετήριο ενός stick. Για κάθε αρχείο: relative path, size, mtime,
  BLAKE3 hash του (κρυπτογραφημένου) blob. Σειριοποιείται (postcard ή CBOR), κρυπτογραφείται,
  και έχει δικό του hash. Ο παραλήπτης το κατεβάζει πρώτο.
- **Ticket**: ό,τι χρειάζεται ο παραλήπτης. Το ticket έχει δημόσιο μέρος (StickId, NodeId,
  address hints, manifest seq και hash ως hint) και μυστικό μέρος (StickKey). Κωδικοποιείται
  σε string (base32) και σε QR, και ως `https://ourstick.com/s/<δημόσιο>#<μυστικό>`. Το μυστικό
  ζει μόνο στο fragment, ποτέ σε query ή path. Ποτέ custom scheme `ourstick://` που κουβαλά κλειδί.
  Ένα ticket ΔΕΝ πρέπει ποτέ να καταγράφεται σε log.
- **Peer**: κόμβος που συμμετέχει σε ένα stick. Ρόλοι: `Owner`, `Reader`, `Seed`.
  Ο cloud/self-hosted seed είναι απλώς ένας Peer με ρόλο Seed. Το πρωτόκολλο δεν τον ξεχωρίζει.
- **Transfer**: μια λήψη με resume. Κατάσταση στο SQLite.

## Τεχνολογίες

- Rust stable, edition 2021.
- Networking: `iroh` + `iroh-blobs` (QUIC, hole punching, relay fallback, content-addressed blobs).
  Πάρε την τελευταία έκδοση με `cargo add iroh iroh-blobs` και διάβασε τα docs.rs πριν γράψεις·
  το API άλλαξε αρκετά μεταξύ εκδόσεων. Μην γράφεις από μνήμη.
- Reference implementation: το `sendme` του iroh (MIT/Apache). Διάβασέ το, μην το αντιγράψεις.
  ΜΗΝ αντιγράψεις κώδικα από AGPL projects (DashBeam/AltSendme, Syncthing).
- Hashing: `blake3`. Crypto: `chacha20poly1305` (AEAD), κλειδιά μέσω `rand`.
- DB: `rusqlite` (bundled). Serialization: `serde` + `postcard`.
- Tauri 2 για apps. UI σε TypeScript (Svelte). Το UI καλεί μόνο Tauri commands που
  καλούν μόνο `ourstick-core`.

## Τι κάνει η v1 (και μόνο αυτά)

- `ourstick share <folder>` → τυπώνει ticket και QR στο τερματικό, μένει ανοιχτό και σερβίρει.
- `ourstick get <ticket> [--out dir]` → κατεβάζει manifest, δείχνει λίστα, κατεβάζει όλα με resume.
- `ourstick ls <ticket>` → δείχνει μόνο τη λίστα αρχείων.
- Ένα stick, ένας owner, N readers, read-only.
- Το stick ενημερώνεται όταν αλλάζει ο φάκελος (watcher) — ο reader βλέπει νέο manifest.
- Δημόσια relays του iroh. Καμία δική μας υποδομή.

## Τι ΔΕΝ κάνει η v1

Μην τα προσθέσεις, ακόμα κι αν φαίνονται εύκολα:
- Δικαιώματα εγγραφής για readers.
- Πολλαπλά sticks σε ένα ticket.
- Cloud seed, billing, accounts.
- Mount ως drive (FUSE, File Provider, Cloud Files API).
- Δικά μας relays.
- Web receiver (λήψη από browser χωρίς app).
- Ανάκληση πρόσβασης εκτός από «άλλαξε κλειδί».
- GUI. Πρώτα CLI. Το Tauri app ξεκινά μόνο όταν το CLI δουλεύει από σπίτι σε laptop με hotspot κινητού.

## Σειρά υλοποίησης

1. `manifest.rs`: περπάτημα φακέλου, BLAKE3 ανά αρχείο, σειριοποίηση. Tests με `tempfile`.
2. `cipher.rs`: trait `Cipher` + `NoCipher` + `ChaChaCipher`. Tests roundtrip.
3. `ticket.rs`: encode/decode. Tests roundtrip + fuzz-ish (κακό input δεν κάνει panic).
4. `store/`: SQLite schema + trait `Store` + `SqliteStore` + `MemoryStore`. Tests.
5. `transport/`: trait `Transport` + `IrohTransport`. Integration test 2 peers.
   Το integration test στην ίδια μηχανή δεν αρκεί. Το βήμα κλείνει με χειροκίνητη δοκιμή
   PC → laptop σε hotspot κινητού, και το CLI τυπώνει αν η σύνδεση είναι direct ή relay.
6. `cli`: share / ls / get.
7. `watcher.rs`.
8. Μετά: Tauri.

Ένα βήμα τη φορά. Κάθε βήμα τελειώνει με `cargo test` πράσινο και `cargo clippy` καθαρό.

## Μαθήματα από προηγούμενο project με μοίρασμα

- Κάθε αποτυχία δικτύου λέει γιατί (π.χ. «δεν βρέθηκε κοινή διαδρομή, μόνο relay»). Καμία σιωπηλή επιστροφή.
- Ένα σημείο αποφασίζει τι λαμβάνει ένας peer. Κενή ή ελλιπής απάντηση σημαίνει άρνηση, ποτέ «όλα».
- Ξένο κείμενο από peers (ονόματα αρχείων, stick) γίνεται escape πριν εμφανιστεί σε τερματικό ή UI.
- Αν η ασφαλής αποθήκευση μυστικών δεν είναι διαθέσιμη, σφάλμα. Ποτέ σιωπηλή πτώση σε απλό αρχείο.
- «Ποιος είναι» και «τι επιτρέπεται» είναι δύο χωριστοί έλεγχοι.
- Λιγότεροι γύροι σχεδιασμού στο χαρτί, περισσότερες δοκιμές σε αληθινές συσκευές και δίκτυα.

## Στυλ

- `cargo fmt` πάντα. `cargo clippy -- -D warnings` πρέπει να περνά.
- Σχόλια και docs στα αγγλικά (το repo είναι δημόσιο). Commits στα αγγλικά, conventional commits.
- Commits απευθείας στο `main`, χωρίς feature branches.
- Το `rust-version` (σήμερα 1.88) δεν ανεβαίνει χωρίς να ρωτήσεις τον Γιώργο. Αν μια εξάρτηση
  το απαιτεί, ρώτα πρώτα ή διάλεξε έκδοση που δεν το απαιτεί. Ενημέρωσε και το README.
- Ονόματα: `snake_case` για functions/modules, `CamelCase` για types, χωρίς συντομογραφίες
  (`manifest`, όχι `mf`).
- Μικρά modules. Αν ένα αρχείο περνά τις 400 γραμμές, σπάσε το.
- Κάθε `unsafe` θέλει σχόλιο `// SAFETY:` και έγκριση από τον Γιώργο.

## Πριν πεις «τελείωσα»

- [ ] `cargo test --workspace` πράσινο
- [ ] `cargo clippy --workspace -- -D warnings` καθαρό
- [ ] `cargo fmt --check` καθαρό
- [ ] Κανένα ticket ή κλειδί σε logs
- [ ] Αν άλλαξε το protocol ή το schema: ενημερώθηκε το `docs/protocol.md` / migration

## Trademark

OurStick είναι εμπορικό σήμα του Γιώργου. Η άδεια Apache-2.0 καλύπτει τον κώδικα, όχι το όνομα.
