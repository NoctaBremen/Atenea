# Guía de desarrollo de Vault

## Requisitos

```bash
sudo pacman -S --needed rust gtk4 mpv yt-dlp
```

Rust 1.92 o posterior (edición 2024). SQLite va incluido en el binario (`rusqlite` con `bundled`,
con FTS5).

## Comprobaciones

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

- `unwrap()` y `expect()` están prohibidos fuera de los tests (lints `deny`), igual que `unsafe`.
- Mensajes y documentación en castellano.
- Registro con `tracing`; nivel con `VAULT_LOG` (`VAULT_LOG=debug`).
- El test de mpv (`tests/reproductor.rs`) se salta solo si faltan mpv, yt-dlp o ffmpeg.

## Mapa del código

La lógica está en la biblioteca y se prueba sin GTK; la interfaz va aparte.

| Módulo | Qué hace |
|---|---|
| `src/discogs/` | Modelo de un disco y lector en streaming del dump (`quick-xml` + `flate2`, memoria constante). `sample.rs` corta muestras |
| `src/db/schema.rs` | Esquema SQLite (versión 2), índices, recuentos (`facets`), bits de formato |
| `src/db/import.rs` | Importador por lotes con filtros, progreso, cancelación y escritura en temporal + rename |
| `src/db/query.rs` | `Filtros` → SQL parametrizado; búsqueda FTS5; estrategia de orden |
| `src/db/base.rs` | Base abierta: ids, filas por páginas, detalle, recuentos, «parecidos» |
| `src/userdb.rs` | Tus datos: marcadores, notas, maletas, búsquedas guardadas (`user.db`, adjunta como `u`) |
| `src/player.rs` | mpv por socket IPC JSON |
| `src/memory.rs` | Plan de memoria (`mmap_size`) y precarga de la base |
| `src/config.rs` | `config.toml`: tema, recientes, sonido, columnas, sesión |
| `src/theme.rs` | Diez temas (del módulo de temas de Digityzer, commit 1f7e585) |
| `src/catalog.rs` | Catálogo género → estilo incluido (`data/catalogo.tsv`) |
| `src/export.rs` | CSV de maletas |
| `src/ui/` | Interfaz GTK4: ventana, modelo perezoso de la lista, filtros, detalle, importación, reproducción, búsqueda en hilo aparte |
| `src/bin/` | `vault-importar`, `vault-muestra`, `vault-catalogo` |

## Decisiones importantes

**Dos bases separadas.** La de Discogs (`bases/*.db`) se abre en solo lectura (`mode=ro`) y se
puede sustituir. Tus datos (`user.db`) se adjuntan como `u` y se enlazan solo por `release_id`.

**La interfaz nunca espera a SQLite.** Las búsquedas van en un hilo con su propia conexión
(`ui/search.rs`). Si llegan varias seguidas, solo se atiende la última y la anterior se
interrumpe (`sqlite3_interrupt`). En el hilo principal solo se leen páginas de 200 filas y el
detalle de un disco, que tardan milisegundos.

**Lista perezosa.** `ui/model.rs` implementa `gio::ListModel` guardando solo los ids del
resultado. La `ColumnView` pide las filas visibles y el modelo las lee por páginas con una caché
pequeña. Un resultado de un millón de discos ocupa lo que sus ids.

**Repintar una fila** saca su página de la caché: la `ColumnView` no repinta si recibe el mismo
objeto. Además, `items_changed` sobre la fila elegida la deselecciona y le quita el foco, así que
la ventana vuelve a poner los dos (`repaint_row`).

**Orden.** Sin filtros (o solo con «Ocultar No me interesa» / «Solo sin marcar», que dejan casi
todo) se recorre el índice de la columna, que ya da la lista ordenada. Con cualquier otro filtro
se filtra y se ordena el resultado, impidiendo el índice con un `+` unario en el `ORDER BY`:
recorrer el índice comprobando cada fila cuesta lo mismo que recorrer toda la base.

**Recuentos del panel** (géneros, estilos, países): se calculan al importar en la tabla `facets`,
para no recorrer la base al abrirla.

**Formatos** (LP, 12", 7"…): bits en `releases.fmt_flags`, en vez de buscar texto fila a fila.

**Memoria.** `memory::plan` mapea la base entera si ocupa como mucho la mitad de la RAM libre y la
precarga en segundo plano; si no, mapea una cuarta parte de la memoria libre sin precarga.

## Rendimiento medido

Con un millón de discos sintéticos (base de 820 MB):

| Operación | Tiempo |
|---|---|
| Importar | 49 s, ~600 MB de RAM como máximo |
| Abrir la base y recuentos del panel | 1 ms |
| Lista entera ordenada por artista | 54–89 ms |
| Un estilo | ~40 ms |
| Formato + años + con vídeos | ~130–170 ms |
| Página de 200 filas | 1–3 ms |

Con el dump real (Electronic, vinilo, 1980–2026): 1,28 millones de discos, 1,7 GB, 8–10 minutos.

Para medir una base: `cargo run --release --example medir -- base.db [--precargar]`.

## Datos de prueba

- `tests/data/fixed_releases.xml`: 9 discos escritos a mano de los que dependen los tests.
- `tests/data/sample_releases.xml`: 2000 discos **sintéticos** con el formato del dump,
  generados con `python3 tools/gen_sample.py` (determinista). Con otros argumentos genera dumps
  grandes para medir: `python3 tools/gen_sample.py 1000000 /tmp/m1_releases.xml`.
- Para usar una muestra real: `vault-muestra discogs_…_releases.xml.gz tests/data/sample_releases.xml 2000`
  (los tests de discos fijos necesitan conservar `fixed_releases.xml` al principio).
- `data/catalogo.tsv` se escribió a mano; se regenera con
  `vault-catalogo discogs_…_releases.xml.gz > data/catalogo.tsv`.

## Probar la interfaz sin pantalla

Con Xvfb y xdotool: `VAULT_AO=null` hace que mpv use la salida de audio nula, y
`XDG_CONFIG_HOME` / `XDG_DATA_HOME` apuntados a una carpeta temporal aíslan la configuración.

## Empaquetado

- **AppImage:** `packaging/build-appimage.sh` → `dist/Vault-x86_64.AppImage`. Usa linuxdeploy y su
  plugin de GTK (GTK4 dentro; mpv y yt-dlp del sistema). Fuerza el tema oscuro por defecto en el
  lanzador del plugin, que si no elige Adwaita claro cuando el escritorio no informa de
  preferencia.
- **Arch:** `packaging/PKGBUILD`.
- Lanzador e icono: `packaging/io.github.noctabremen.Vault.desktop` y `.svg`; el PNG de
  `docs/img/icono.png` sale del SVG.

## Publicar una versión

1. Subir `version` en `Cargo.toml` y añadir la entrada en `CHANGELOG.md`.
2. `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`.
3. Commit, etiqueta `vX.Y.Z` y push de la rama y la etiqueta.
4. `packaging/build-appimage.sh` y subir `dist/Vault-x86_64.AppImage` a la versión en GitHub.
