# Vault

Explorador de vinilos sobre los dumps de Discogs. Rust + GTK4, minimalista, con los mismos temas
que Spector y Digityzer (Dracula por defecto).

- Importas una vez el dump mensual de releases de [data.discogs.com](https://data.discogs.com/)
  a SQLite, eligiendo qué entra (formato, géneros → estilos, años).
- Buscas y filtras al instante entre millones de discos.
- Escuchas los vídeos de YouTube de cada disco con mpv + yt-dlp (solo audio o con vídeo).
- Marcas lo que te interesa, lo metes en maletas y exportas a CSV para comprar.

## Instalar

Dependencias (Arch):

```bash
sudo pacman -S --needed gtk4 mpv yt-dlp rust
```

Compilar y ejecutar:

```bash
cargo build --release
./target/release/vault
```

O con el PKGBUILD de `packaging/` (instala también `.desktop` e icono):

```bash
cd packaging && makepkg -si
```

## Primer uso

1. Descarga `discogs_AAAAMMDD_releases.xml.gz` de data.discogs.com (no hace falta descomprimirlo).
2. En Vault: **Bases → Importar dump…**. Por defecto viene el preajuste **«Todo en vinilo»**
   (todos los géneros, solo vinilo), que es el uso recomendado. Para probar antes, crea una
   base pequeña marcando un solo estilo (p. ej. Electronic → Tech House).
3. La importación va en segundo plano con progreso y se puede cancelar. Al acabar, la base se
   abre sola y queda en **Bases → Recientes**.

Desde la terminal también se puede importar:

```bash
vault-importar discogs_20261101_releases.xml.gz ~/.local/share/vault/bases/todo-en-vinilo.db \
  --nombre "Todo en vinilo" --formato Vinyl

vault-importar discogs_20261101_releases.xml.gz tech-house.db \
  --estilo "Electronic/Tech House" --formato Vinyl --desde 1995 --hasta 2005
```

## Atajos

| Tecla | Acción |
|---|---|
| Espacio | Play/pausa (si no suena nada, pone el disco elegido) |
| ← / → | −10 s / +10 s |
| , / . | Vídeo anterior / siguiente del disco |
| 1 2 3 4 | Me interesa · No me interesa · Escuchado · Volver a escuchar (otra vez = quitar; 0 quita) |
| N / P | Disco siguiente / anterior |
| D | Modo digging: recorre la lista en cola; 1–4 marcan **y pasan al siguiente** |
| V | Mostrar u ocultar el vídeo |
| Alt+1…9 | Meter el disco elegido en la maleta 1…9 |
| Ctrl+F | Ir al buscador |
| Doble clic / Intro | Escuchar el disco |

En el buscador, `"frase exacta"` busca la frase y las palabras sueltas buscan por prefijo
(`kalo axis` encuentra «Kalo Tren – Axis Point»). Busca en artista, título, sello, catno,
pistas y también en tus notas.

Los discos se pueden **arrastrar** desde la lista a una maleta del panel izquierdo.

En el detalle de un disco, **Parecidos** (mismos estilos, ±2 años), **Más del sello** y **Más del
artista** rellenan los filtros y buscan.

## Vídeo flotante en Hyprland / mango

La ventana de vídeo de mpv usa la clase / app-id `vault-video`:

```
windowrulev2 = float, class:^(vault-video)$
windowrulev2 = size 480 270, class:^(vault-video)$
windowrulev2 = move 100%-500 100%-300, class:^(vault-video)$
```

## Dónde se guarda cada cosa

| Qué | Dónde |
|---|---|
| Ajustes (tema, sonido, recientes) | `~/.config/vault/config.toml` |
| Bases de Discogs | `~/.local/share/vault/bases/*.db` (se puede cambiar en Ajustes) |
| Tus datos: marcadores, notas, maletas, búsquedas | `~/.local/share/vault/user.db` |

Una base importada con una versión anterior del esquema pide volver a importar el dump (tus datos
no se tocan). Tus datos van aparte y se enlazan por `release_id`: sirven para todas las bases y no se pierden
si borras o vuelves a importar una base. Las bases de Discogs se abren en solo lectura.

## Herramientas

- `vault-importar`: importar sin interfaz (ver arriba).
- `vault-muestra <dump.xml.gz> <salida.xml> [n]`: los primeros n discos (2000) de un dump, para
  los tests.
- `vault-catalogo <dump.xml.gz> > data/catalogo.tsv`: regenera el catálogo género → estilo que
  usa el diálogo de importación.
- `cargo run --release --example medir -- base.db`: tiempos de búsqueda sobre una base.

## Desarrollo

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

- La lógica (parser, importador, consultas, base de usuario, mpv, CSV) está en la biblioteca
  `src/` y se prueba sin GTK; la interfaz está en `src/ui/`.
- `tests/data/sample_releases.xml` es una muestra **sintética** con el formato del dump
  (`tools/gen_sample.py`); los tests dependen solo de los 9 discos fijos de
  `tests/data/fixed_releases.xml`. Para sustituirla por una real:
  `vault-muestra discogs_…_releases.xml.gz tests/data/sample_releases.xml 2000`.
- `data/catalogo.tsv` es provisional (escrito a mano): conviene regenerarlo con
  `vault-catalogo` sobre un dump real.
- El tema de color viene del módulo de temas de Digityzer (commit 1f7e585).

Registro: `VAULT_LOG=debug vault`.
