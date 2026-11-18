//! Modelo de un disco tal como viene en el dump de releases de Discogs.

/// Artista de un disco (`<artists><artist>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Artist {
    /// Id de artista en Discogs.
    pub id: u64,
    /// Nombre (con el sufijo de desambiguación, p. ej. «Nombre (2)»).
    pub name: String,
    /// Variación del nombre usada en este disco, si la hay.
    pub anv: String,
    /// Texto que une con el siguiente artista («&», «vs.», «,»…).
    pub join: String,
}

impl Artist {
    /// Nombre que se muestra: la variación si existe; si no, el nombre sin «(2)».
    pub fn display_name(&self) -> &str {
        if !self.anv.is_empty() {
            return &self.anv;
        }
        strip_disambiguation(&self.name)
    }
}

/// Quita el sufijo « (n)» que Discogs añade a los nombres repetidos.
pub fn strip_disambiguation(name: &str) -> &str {
    if let Some(open) = name.rfind(" (")
        && name.ends_with(')')
        && name[open + 2..name.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit())
        && name.len() > open + 3
    {
        return &name[..open];
    }
    name
}

/// Línea de artistas unida con sus `join`: «A & B», «A vs. B».
pub fn join_artists(artists: &[Artist]) -> String {
    let mut s = String::new();
    for (i, a) in artists.iter().enumerate() {
        s.push_str(a.display_name());
        if i + 1 < artists.len() {
            let j = a.join.trim();
            if j.is_empty() || j == "," {
                s.push_str(", ");
            } else {
                s.push(' ');
                s.push_str(j);
                s.push(' ');
            }
        }
    }
    s
}

/// Crédito (`<extraartists><artist>`): productor, remezcla, diseño…
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credit {
    /// Id de artista.
    pub id: u64,
    /// Nombre.
    pub name: String,
    /// Papel («Producer», «Remix»…).
    pub role: String,
    /// Pistas a las que se aplica («A1, B2»), vacío si es todo el disco.
    pub tracks: String,
}

/// Sello y número de catálogo (`<labels><label>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelRef {
    /// Id de sello.
    pub id: u64,
    /// Nombre del sello.
    pub name: String,
    /// Número de catálogo.
    pub catno: String,
}

/// Formato (`<formats><format>`): «Vinyl» con descripciones «12"», «33 ⅓ RPM», «EP»…
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Format {
    /// Nombre («Vinyl», «CD», «File»…).
    pub name: String,
    /// Cantidad (nº de discos).
    pub qty: u32,
    /// Texto libre («Red», «Test Pressing»…).
    pub text: String,
    /// Descripciones.
    pub descriptions: Vec<String>,
}

/// Pista de la tracklist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    /// Posición («A1», «B»…).
    pub position: String,
    /// Título.
    pub title: String,
    /// Duración tal como viene («4:45»), puede estar vacía.
    pub duration: String,
}

/// Vídeo enlazado (normalmente de YouTube).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Video {
    /// URL.
    pub url: String,
    /// Título.
    pub title: String,
    /// Duración en segundos (0 si no consta).
    pub duration: u32,
}

/// Un disco del dump.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// Id de release en Discogs.
    pub id: u64,
    /// Estado («Accepted», «Draft»…).
    pub status: String,
    /// Título.
    pub title: String,
    /// Artistas principales.
    pub artists: Vec<Artist>,
    /// Créditos.
    pub credits: Vec<Credit>,
    /// Sellos.
    pub labels: Vec<LabelRef>,
    /// Formatos.
    pub formats: Vec<Format>,
    /// Géneros.
    pub genres: Vec<String>,
    /// Estilos.
    pub styles: Vec<String>,
    /// País.
    pub country: String,
    /// Fecha de publicación tal como viene («1999-03-00», «1999»).
    pub released: String,
    /// Notas.
    pub notes: String,
    /// Id del master, si lo tiene.
    pub master_id: Option<u64>,
    /// Tracklist (solo pistas de primer nivel).
    pub tracks: Vec<Track>,
    /// Vídeos.
    pub videos: Vec<Video>,
}

impl Release {
    /// Año de publicación, si `released` empieza por cuatro cifras distintas de 0000.
    pub fn year(&self) -> Option<u16> {
        let y = self.released.get(..4)?;
        if !y.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        y.parse().ok().filter(|&y| y > 0)
    }

    /// Línea de artistas.
    pub fn artist_line(&self) -> String {
        join_artists(&self.artists)
    }

    /// ¿Tiene algún formato con ese nombre (sin distinguir mayúsculas)?
    pub fn has_format(&self, name: &str) -> bool {
        self.formats
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desambiguacion_y_union() {
        assert_eq!(strip_disambiguation("Nombre (2)"), "Nombre");
        assert_eq!(strip_disambiguation("Nombre (UK)"), "Nombre (UK)");
        assert_eq!(strip_disambiguation("()"), "()");
        let a = |n: &str, j: &str| Artist {
            name: n.into(),
            join: j.into(),
            ..Default::default()
        };
        assert_eq!(join_artists(&[a("A (3)", "&"), a("B", "")]), "A & B");
        assert_eq!(join_artists(&[a("A", ","), a("B", "")]), "A, B");
        let anv = Artist {
            name: "X".into(),
            anv: "Equis".into(),
            ..Default::default()
        };
        assert_eq!(anv.display_name(), "Equis");
    }

    #[test]
    fn anyo() {
        let r = |s: &str| Release {
            released: s.into(),
            ..Default::default()
        };
        assert_eq!(r("1999-03-00").year(), Some(1999));
        assert_eq!(r("1999").year(), Some(1999));
        assert_eq!(r("0000").year(), None);
        assert_eq!(r("").year(), None);
        assert_eq!(r("19xx").year(), None);
    }
}
