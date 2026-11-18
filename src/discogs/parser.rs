//! Lector en streaming del dump de releases (T04): `quick-xml` sobre cualquier `BufRead` (el
//! `.gz` se descomprime al vuelo con `flate2`), emite un [`Release`] cada vez y la memoria no
//! crece con el tamaño del archivo.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use flate2::read::MultiGzDecoder;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::model::{Artist, Credit, Format, LabelRef, Release, Track, Video};

/// Error al leer el dump.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Error de E/S.
    #[error("error de lectura: {0}")]
    Io(#[from] std::io::Error),
    /// XML mal formado.
    #[error("XML no válido cerca del byte {pos}: {source}")]
    Xml {
        /// Posición aproximada.
        pos: u64,
        /// Error de quick-xml.
        source: quick_xml::Error,
    },
}

/// Abre un dump (`.xml.gz` o `.xml`) y devuelve el lector de discos.
pub fn open_dump(path: &Path) -> Result<ReleaseReader<Box<dyn BufRead + Send>>, ParseError> {
    let file = File::open(path)?;
    let gz = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
    let inner: Box<dyn BufRead + Send> = if gz {
        Box::new(BufReader::with_capacity(
            1 << 20,
            MultiGzDecoder::new(BufReader::with_capacity(1 << 20, file)),
        ))
    } else {
        Box::new(BufReader::with_capacity(1 << 20, file))
    };
    Ok(ReleaseReader::new(inner))
}

/// Iterador de discos sobre un XML de releases.
pub struct ReleaseReader<R: BufRead> {
    reader: Reader<R>,
    buf: Vec<u8>,
    done: bool,
}

impl<R: BufRead> ReleaseReader<R> {
    /// Lector sobre XML ya descomprimido.
    pub fn new(inner: R) -> Self {
        let mut reader = Reader::from_reader(inner);
        reader.config_mut().trim_text(false);
        Self {
            reader,
            buf: Vec::with_capacity(64 * 1024),
            done: false,
        }
    }

    /// Bytes de XML leídos hasta ahora (para el progreso).
    pub fn position(&self) -> u64 {
        self.reader.buffer_position()
    }

    /// Lee hasta el siguiente `<release>` de primer nivel. `None` al final.
    fn next_release(&mut self) -> Result<Option<Release>, ParseError> {
        loop {
            self.buf.clear();
            let ev = match self.reader.read_event_into(&mut self.buf) {
                Ok(ev) => ev,
                Err(e) => return Err(xml_err(self.reader.buffer_position(), e)),
            };
            match ev {
                Event::Start(e) if e.local_name().as_ref() == "release" => {
                    let mut rel = Release::default();
                    read_release_attrs(&e, &mut rel);
                    self.read_body(&mut rel)?;
                    return Ok(Some(rel));
                }
                Event::Empty(e) if e.local_name().as_ref() == "release" => {
                    let mut rel = Release::default();
                    read_release_attrs(&e, &mut rel);
                    return Ok(Some(rel));
                }
                Event::Eof => return Ok(None),
                _ => {}
            }
        }
    }

    /// Lee el contenido de un `<release>` hasta su cierre.
    fn read_body(&mut self, rel: &mut Release) -> Result<(), ParseError> {
        let mut path: Vec<String> = Vec::with_capacity(8);
        let mut text = String::new();
        loop {
            self.buf.clear();
            let ev = match self.reader.read_event_into(&mut self.buf) {
                Ok(ev) => ev,
                Err(e) => return Err(xml_err(self.reader.buffer_position(), e)),
            };
            match ev {
                Event::Start(e) => {
                    let name = e.local_name().as_ref().to_owned();
                    path.push(name);
                    text.clear();
                    on_start(rel, &path, &e);
                }
                Event::Empty(e) => {
                    let name = e.local_name().as_ref().to_owned();
                    path.push(name);
                    text.clear();
                    on_start(rel, &path, &e);
                    on_end(rel, &path, "");
                    path.pop();
                }
                Event::Text(t) => text.push_str(&t.xml10_content()),
                Event::CData(t) => text.push_str(&t.xml10_content()),
                Event::GeneralRef(r) => {
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        text.push(c);
                    } else {
                        let name: &str = &r;
                        text.push_str(match name {
                            "amp" => "&",
                            "lt" => "<",
                            "gt" => ">",
                            "quot" => "\"",
                            "apos" => "'",
                            _ => "",
                        });
                    }
                }
                Event::End(_) => {
                    if path.is_empty() {
                        return Ok(()); // </release>
                    }
                    on_end(rel, &path, text.trim());
                    text.clear();
                    path.pop();
                }
                Event::Eof => return Ok(()),
                _ => {}
            }
        }
    }
}

impl<R: BufRead> Iterator for ReleaseReader<R> {
    type Item = Result<Release, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.next_release() {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

fn xml_err(pos: u64, source: quick_xml::Error) -> ParseError {
    ParseError::Xml { pos, source }
}

fn attr(e: &BytesStart<'_>, name: &str) -> String {
    e.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.trim().to_owned())
        })
        .unwrap_or_default()
}

fn read_release_attrs(e: &BytesStart<'_>, rel: &mut Release) {
    rel.id = attr(e, "id").parse().unwrap_or(0);
    rel.status = attr(e, "status");
}

/// Ruta relativa al `<release>` como `&[&str]` para poder hacer `match`.
fn p<'a>(path: &'a [String], buf: &'a mut [&'a str; 6]) -> &'a [&'a str] {
    let n = path.len().min(buf.len());
    for (slot, s) in buf.iter_mut().zip(path.iter()) {
        *slot = s.as_str();
    }
    &buf[..n]
}

fn on_start(rel: &mut Release, path: &[String], e: &BytesStart<'_>) {
    let mut b = [""; 6];
    match p(path, &mut b) {
        ["artists", "artist"] => rel.artists.push(Artist::default()),
        ["extraartists", "artist"] => rel.credits.push(Credit::default()),
        ["labels", "label"] => rel.labels.push(LabelRef {
            id: attr(e, "id").parse().unwrap_or(0),
            name: attr(e, "name"),
            catno: attr(e, "catno"),
        }),
        ["formats", "format"] => rel.formats.push(Format {
            name: attr(e, "name"),
            qty: attr(e, "qty").parse().unwrap_or(0),
            text: attr(e, "text"),
            descriptions: Vec::new(),
        }),
        ["tracklist", "track"] => rel.tracks.push(Track::default()),
        ["videos", "video"] => rel.videos.push(Video {
            url: attr(e, "src"),
            title: String::new(),
            duration: attr(e, "duration").parse().unwrap_or(0),
        }),
        _ => {}
    }
}

fn on_end(rel: &mut Release, path: &[String], text: &str) {
    let mut b = [""; 6];
    let t = || text.to_owned();
    match p(path, &mut b) {
        ["title"] => rel.title = t(),
        ["country"] => rel.country = t(),
        ["released"] => rel.released = t(),
        ["notes"] => rel.notes = t(),
        ["master_id"] => rel.master_id = text.parse().ok(),
        ["genres", "genre"] if !text.is_empty() => rel.genres.push(t()),
        ["styles", "style"] if !text.is_empty() => rel.styles.push(t()),
        ["artists", "artist", field] => {
            if let Some(a) = rel.artists.last_mut() {
                match *field {
                    "id" => a.id = text.parse().unwrap_or(0),
                    "name" => a.name = t(),
                    "anv" => a.anv = t(),
                    "join" => a.join = t(),
                    _ => {}
                }
            }
        }
        ["extraartists", "artist", field] => {
            if let Some(c) = rel.credits.last_mut() {
                match *field {
                    "id" => c.id = text.parse().unwrap_or(0),
                    "name" => c.name = t(),
                    "role" => c.role = t(),
                    "tracks" => c.tracks = t(),
                    _ => {}
                }
            }
        }
        ["formats", "format", "descriptions", "description"] if !text.is_empty() => {
            if let Some(f) = rel.formats.last_mut() {
                f.descriptions.push(t());
            }
        }
        ["tracklist", "track", field] => {
            if let Some(tr) = rel.tracks.last_mut() {
                match *field {
                    "position" => tr.position = t(),
                    "title" => tr.title = t(),
                    "duration" => tr.duration = t(),
                    _ => {}
                }
            }
        }
        ["videos", "video", "title"] => {
            if let Some(v) = rel.videos.last_mut() {
                v.title = t();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<releases>
<release id="7" status="Accepted"><artists><artist><id>1</id><name>Uno (2)</name><anv></anv><join>&amp;</join><role></role><tracks></tracks></artist><artist><id>2</id><name>Dos</name><anv>Do</anv><join></join></artist></artists><title>Caf&#233; &amp; Noche</title><labels><label name="Sello" catno="SL 001" id="9"/></labels><extraartists><artist><id>3</id><name>Prod</name><role>Producer</role><tracks>A1</tracks></artist></extraartists><formats><format name="Vinyl" qty="1" text=""><descriptions><description>12"</description><description>33 ⅓ RPM</description></descriptions></format></formats><genres><genre>Electronic</genre></genres><styles><style>Techno</style><style>Tech House</style></styles><country>Germany</country><released>1997-05-00</released><notes><![CDATA[Nota <b>cruda</b>]]></notes><master_id is_main_release="true">55</master_id><tracklist><track><position>A1</position><title>Pista</title><duration>6:01</duration><artists><artist><id>4</id><name>Otro</name></artist></artists></track><track><position>B</position><title>Otra</title><duration></duration><sub_tracks><track><position>B.a</position><title>Sub</title></track></sub_tracks></track></tracklist><videos><video src="https://www.youtube.com/watch?v=abc" duration="361" embed="true"><title>Uno - Pista</title><description>x</description></video></videos></release>
<release id="8" status="Accepted"><title>Vacío</title></release>
</releases>"#;

    #[test]
    fn lee_todos_los_campos() {
        let rels: Vec<Release> = ReleaseReader::new(XML.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rels.len(), 2);
        let r = &rels[0];
        assert_eq!(r.id, 7);
        assert_eq!(r.title, "Café & Noche");
        assert_eq!(r.artist_line(), "Uno & Do");
        assert_eq!(r.artists.len(), 2, "los artistas de pista no se mezclan");
        assert_eq!(r.labels[0].catno, "SL 001");
        assert_eq!(r.credits[0].role, "Producer");
        assert_eq!(r.formats[0].descriptions, ["12\"", "33 ⅓ RPM"]);
        assert_eq!(r.styles, ["Techno", "Tech House"]);
        assert_eq!(r.year(), Some(1997));
        assert_eq!(r.notes, "Nota <b>cruda</b>");
        assert_eq!(r.master_id, Some(55));
        assert_eq!(
            r.tracks.len(),
            2,
            "las subpistas no son pistas de primer nivel"
        );
        assert_eq!(r.tracks[0].duration, "6:01");
        assert_eq!(r.tracks[1].title, "Otra");
        assert_eq!(r.videos[0].duration, 361);
        assert_eq!(r.videos[0].title, "Uno - Pista");
        assert_eq!(rels[1].title, "Vacío");
    }

    #[test]
    fn xml_roto_da_error_y_termina() {
        let mut it =
            ReleaseReader::new(&b"<releases><release id=\"1\"><title>x</titl></release>"[..]);
        assert!(matches!(it.next(), Some(Err(_))));
        assert!(it.next().is_none());
    }
}
