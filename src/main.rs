use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Datelike;
use clap::Parser;
use musicbrainz_rs::entity::artist_credit::ArtistCredit;
use musicbrainz_rs::entity::release::Release;
use musicbrainz_rs::entity::CoverartResponse;
use musicbrainz_rs::{Fetch, FetchCoverart};

const USER_AGENT: &str = "musicbrainz_cuesheet/0.1.0 (alpha testing)";
const COVER_ART_PATH_COMPONENT: &str = "Cover";

#[derive(Parser)]
struct Args {
    #[clap(short = 'r', long)]
    release_id: String,

    #[clap(short = 'c', long)]
    cover_art: bool,

    #[clap(short = 'o', long)]
    out_dir: PathBuf,
}

fn join_artists(artists: &[ArtistCredit]) -> String {
    artists
        .iter()
        .map(|a| format!("{}{}", a.name, a.joinphrase.clone().unwrap_or_default()))
        .collect::<String>()
}

fn millisecond_to_mmssff(ms: u32) -> String {
    // From https://wiki.hydrogenaud.io/index.php?title=Cue_sheet:
    // FF the number of frames (there are seventy five frames to one second)
    const MILLISECONDS_PER_FRAME: f64 = 1000.0 / 75.0;

    let ms_part = (ms as f64) % 1000.0;
    let frames = (ms_part / MILLISECONDS_PER_FRAME).round() as u32;
    let seconds = (ms / 1000) % 60;
    let minutes = ms / 60000;

    format!("{minutes:02}:{seconds:02}:{frames:02}")
}

fn download_cover_art(url: &str, output_path_prefix: &Path) {
    let resp = reqwest::blocking::get(url).unwrap();
    if resp.status().is_success() {
        let file_extension = Path::new(resp.url().path()).extension().unwrap().to_string_lossy();
        let output_path = output_path_prefix.with_extension(file_extension.as_ref());
        std::fs::write(output_path, resp.bytes().unwrap()).unwrap();
    } else {
        eprintln!("HTTP error code {}", resp.status());
    }
}

fn main() {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out_dir).unwrap();

    musicbrainz_rs::config::set_user_agent(USER_AGENT);

    let release = Release::fetch()
        .id(&args.release_id)
        .with_artist_credits()
        .with_genres()
        .with_labels()
        .with_recordings()
        .with_release_groups()
        .execute()
        .unwrap();
    let mut release_cuesheet = String::new();
    //std::fs::write("D:\\mb_debug.txt", format!("{:#?}", release));

    if let Some(artists) = release.artist_credit {
        writeln!(release_cuesheet, "PERFORMER \"{}\"", join_artists(&artists)).unwrap();
    }

    if let Some(release_group) = release.release_group {
        if let Some(genres) = release_group.genres {
            writeln!(release_cuesheet, "REM GENRE {}", genres.into_iter().map(|g| g.name).collect::<Box<_>>().join("; ")).unwrap();
        }

        if let Some(release_date) = release_group.first_release_date {
            // currently musicbrainz_rs always converts incomplete date (e.g. without month and/or day) to NaiveDate,
            // filling the missing part with value "01"
            writeln!(
                release_cuesheet,
                "REM DATE {}",
                if release_date.ordinal() == 1 {
                    release_date.year().to_string()
                } else {
                    release_date.to_string()
                }
            )
            .unwrap();
        }
    }

    if let Some(label) = release.label_info {
        for l in label.into_iter().filter_map(|li| li.label) {
            if !l.name.is_empty() {
                writeln!(release_cuesheet, "REM COMMENT \"{}\"", l.name).unwrap();
            }
        }
    }

    writeln!(release_cuesheet, "REM MUSICBRAINZ_ALBUM_ID {}", release.id).unwrap();
    writeln!(release_cuesheet, "FILE \"CDImage.flac\" WAVE").unwrap();

    if let Some(media) = release.media {
        let is_album = media.len() > 1;
        for medium in media {
            let medium_id = format!("{} {:02}", medium.format.unwrap_or_default(), medium.position.unwrap_or_default());

            let mut medium_title = format!("TITLE \"{}", release.title);
            if is_album {
                medium_title += &format!("- {medium_id}")
            }
            if let Some(t) = medium.title {
                if !t.is_empty() {
                    medium_title += &format!(": {}", t);
                }
            }
            medium_title += "\"";

            let mut medium_cuesheet = format!("{medium_title}\n{release_cuesheet}");

            let mut track_start = 0;
            for track in medium.tracks.unwrap_or_default().iter() {
                writeln!(medium_cuesheet, "  TRACK {:02} AUDIO", track.position).unwrap();
                writeln!(medium_cuesheet, "    TITLE \"{}\"", track.title).unwrap();

                if let Some(track_artists) = &track.recording.artist_credit {
                    writeln!(medium_cuesheet, "    PERFORMER \"{}\"", join_artists(track_artists)).unwrap();
                }

                let track_length = track.length.unwrap();
                writeln!(medium_cuesheet, "    INDEX 01 {}", millisecond_to_mmssff(track_start)).unwrap();
                track_start += track_length;
            }

            let output_filename = format!("{medium_id}.cue");
            std::fs::write(args.out_dir.join(output_filename), medium_cuesheet).unwrap();
        }
    }

    if args.cover_art {
        let cover_art_path = args.out_dir.join(COVER_ART_PATH_COMPONENT);
        if let Ok(resp) = Release::fetch_coverart().id(&args.release_id).execute() {
            match resp {
                CoverartResponse::Url(cover_art_url) => {
                    download_cover_art(&cover_art_url, &cover_art_path);
                }
                CoverartResponse::Json(cover_art) => {
                    std::fs::create_dir_all(&cover_art_path).unwrap();

                    for img in cover_art.images {
                        let img_filename_stem = img.types.iter().map(|t| format!("{t:#?}")).collect::<Box<_>>().join("_");
                        download_cover_art(&img.image, &cover_art_path.join(img_filename_stem));
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        } else {
            eprintln!("Failed to download cover art")
        }
    }
}
