# Cambios

## 1.0.0 — 2026-11-18

Primera versión.

### Importar
- Lectura en streaming del dump de releases de Discogs (`.xml.gz`) sin descomprimirlo, con
  memoria constante.
- Filtros al importar, todos combinables: formato, géneros completos o estilos sueltos (árbol
  género → estilo) y rango de años. Preajuste «Todo en vinilo».
- Progreso con discos por segundo y tiempo restante, cancelación sin dejar bases a medias.
- Varias bases con nombre y cambio rápido entre ellas; se reabre la última.
- `vault-importar` para importar desde la terminal.

### Buscar
- Búsqueda de texto (FTS5) con frase exacta y prefijos, sin distinguir acentos; también busca en
  tus notas.
- Filtros por artista, sello, años, país, géneros, estilos, formato, con vídeos, marcadores,
  «Solo sin marcar», «Ocultar No me interesa» y maleta.
- Lista perezosa para millones de discos, búsquedas en segundo plano, orden por columnas.
- Columnas de ancho estable, ocultables y recordadas.
- Búsquedas guardadas con nombre y sesión restaurada al abrir (filtros, orden y disco).
- «Parecidos», «Más del sello» y «Más del artista» desde el detalle.

### Escuchar y clasificar
- Reproducción con mpv + yt-dlp: solo audio o ventana de vídeo pequeña (clase `vault-video`),
  PipeWire por defecto, PulseAudio o ALSA con dispositivo.
- Modo digging: recorre los resultados en cola; 1–4 marcan y pasan al siguiente.
- Marcadores, maletas (arrastrar, Alt+1–9), notas por disco y exportación CSV.

### Interfaz
- Diez temas oscuros (Dracula por defecto), panel de detalle movible, ventana «Acerca de».
- La base se carga en RAM si cabe holgadamente, para búsquedas rápidas desde el principio.

### Empaquetado
- AppImage (GTK4 incluido), PKGBUILD para Arch, lanzador `.desktop` e icono.
