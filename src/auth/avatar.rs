use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

pub const HEAD_SIZE: u32 = 8;
pub const HEAD_BYTES: usize = (HEAD_SIZE * HEAD_SIZE * 4) as usize;

const FACE_ORIGIN: (u32, u32) = (8, 8);
const HAT_ORIGIN: (u32, u32) = (40, 8);
const MIN_SKIN_WIDTH: u32 = 64;
const MIN_SKIN_HEIGHT: u32 = 32;

struct Bitmap {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Bitmap {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * self.width + x) * 4) as usize;

        [
            self.rgba[offset],
            self.rgba[offset + 1],
            self.rgba[offset + 2],
            self.rgba[offset + 3],
        ]
    }
}

fn decode(png_bytes: &[u8]) -> anyhow::Result<Bitmap> {
    let mut decoder = png::Decoder::new(png_bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

    let mut reader = decoder
        .read_info()
        .context("스킨 이미지를 읽지 못했어요.")?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .context("스킨 이미지를 해석하지 못했어요.")?;

    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => bail!("지원하지 않는 스킨 이미지 형식이에요: {other:?}"),
    };

    let pixels = (info.width * info.height) as usize;
    let mut rgba = Vec::with_capacity(pixels * 4);

    for index in 0..pixels {
        let offset = index * channels;
        rgba.extend_from_slice(&buffer[offset..offset + 3]);
        rgba.push(if channels == 4 { buffer[offset + 3] } else { 255 });
    }

    Ok(Bitmap {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn blend(base: [u8; 4], over: [u8; 4]) -> [u8; 4] {
    if over[3] == 0 {
        return base;
    }
    if over[3] == 255 {
        return over;
    }

    let alpha = over[3] as u32;
    let inverse = 255 - alpha;
    let mix = |a: u8, b: u8| (((a as u32 * alpha) + (b as u32 * inverse)) / 255) as u8;

    [
        mix(over[0], base[0]),
        mix(over[1], base[1]),
        mix(over[2], base[2]),
        base[3].max(over[3]),
    ]
}

pub fn head_from_skin(png_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let skin = decode(png_bytes)?;

    if skin.width < MIN_SKIN_WIDTH || skin.height < MIN_SKIN_HEIGHT {
        bail!(
            "스킨 이미지 크기가 예상과 달라요: {}x{}",
            skin.width,
            skin.height
        );
    }

    let mut head = Vec::with_capacity(HEAD_BYTES);

    for y in 0..HEAD_SIZE {
        for x in 0..HEAD_SIZE {
            let face = skin.pixel(FACE_ORIGIN.0 + x, FACE_ORIGIN.1 + y);
            let hat = skin.pixel(HAT_ORIGIN.0 + x, HAT_ORIGIN.1 + y);

            head.extend_from_slice(&blend(face, hat));
        }
    }

    Ok(head)
}

pub fn cache_path(cache_dir: &Path, uuid: &str) -> PathBuf {
    cache_dir.join("skins").join(format!("{uuid}.rgba"))
}

pub fn load_cached(cache_dir: &Path, uuid: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cache_path(cache_dir, uuid)).ok()?;

    (bytes.len() == HEAD_BYTES).then_some(bytes)
}

fn store(cache_dir: &Path, uuid: &str, head: &[u8]) -> anyhow::Result<()> {
    let path = cache_path(cache_dir, uuid);
    let parent = path.parent().context("스킨 캐시 경로가 올바르지 않아요.")?;

    std::fs::create_dir_all(parent)
        .with_context(|| format!("{} 폴더를 만들지 못했어요.", parent.display()))?;
    std::fs::write(&path, head)
        .with_context(|| format!("{} 에 쓰지 못했어요.", path.display()))?;

    Ok(())
}

pub fn fetch_head(cache_dir: &Path, uuid: &str, skin_url: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(cached) = load_cached(cache_dir, uuid) {
        return Ok(cached);
    }

    let response = crate::http::send(crate::http::client()?.get(skin_url))
        .context("스킨을 내려받지 못했어요.")?;
    let bytes = response.bytes().context("스킨 데이터를 읽지 못했어요.")?;

    let head = head_from_skin(&bytes)?;

    if let Err(error) = store(cache_dir, uuid, &head) {
        log::warn!("스킨 캐시 저장 실패: {error:#}");
    }

    Ok(head)
}

pub fn to_image(head: &[u8]) -> slint::Image {
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(HEAD_SIZE, HEAD_SIZE);

    if head.len() == HEAD_BYTES {
        buffer.make_mut_bytes().copy_from_slice(head);
    }

    slint::Image::from_rgba8(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(rgba).unwrap();
        }
        out
    }

    fn skin(face: [u8; 4], hat: [u8; 4], width: u32, height: u32) -> Vec<u8> {
        let mut rgba = vec![0_u8; (width * height * 4) as usize];

        let mut put = |x: u32, y: u32, color: [u8; 4]| {
            let offset = ((y * width + x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&color);
        };

        for y in 0..HEAD_SIZE {
            for x in 0..HEAD_SIZE {
                put(FACE_ORIGIN.0 + x, FACE_ORIGIN.1 + y, face);
                put(HAT_ORIGIN.0 + x, HAT_ORIGIN.1 + y, hat);
            }
        }

        encode(width, height, &rgba)
    }

    #[test]
    fn extracts_an_eight_by_eight_head() {
        let png = skin([10, 20, 30, 255], [0, 0, 0, 0], 64, 64);
        let head = head_from_skin(&png).unwrap();

        assert_eq!(head.len(), HEAD_BYTES);
        assert_eq!(&head[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn an_opaque_hat_replaces_the_face() {
        let png = skin([10, 20, 30, 255], [200, 100, 50, 255], 64, 64);
        let head = head_from_skin(&png).unwrap();

        assert_eq!(&head[0..4], &[200, 100, 50, 255]);
    }

    #[test]
    fn a_transparent_hat_leaves_the_face_alone() {
        let png = skin([10, 20, 30, 255], [255, 255, 255, 0], 64, 64);
        let head = head_from_skin(&png).unwrap();

        assert_eq!(&head[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn a_half_transparent_hat_is_blended() {
        let png = skin([0, 0, 0, 255], [255, 255, 255, 128], 64, 64);
        let head = head_from_skin(&png).unwrap();

        assert!(head[0] > 100 && head[0] < 160, "got {}", head[0]);
        assert_eq!(head[3], 255);
    }

    #[test]
    fn legacy_sixty_four_by_thirty_two_skins_work() {
        let png = skin([1, 2, 3, 255], [0, 0, 0, 0], 64, 32);
        let head = head_from_skin(&png).unwrap();

        assert_eq!(&head[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn undersized_images_are_rejected() {
        let png = encode(16, 16, &vec![0_u8; 16 * 16 * 4]);

        assert!(head_from_skin(&png).is_err());
    }

    #[test]
    fn garbage_input_is_rejected() {
        assert!(head_from_skin(b"not a png").is_err());
    }

    #[test]
    fn the_cache_path_is_namespaced_by_uuid() {
        let path = cache_path(Path::new("/cache"), "069a79f4-44e9-4726-a5be-fca90e38aaf5");

        assert!(path.ends_with("skins/069a79f4-44e9-4726-a5be-fca90e38aaf5.rgba"));
    }

    #[test]
    fn a_truncated_cache_entry_is_ignored() {
        let dir = std::env::temp_dir().join(format!("erkuia-avatar-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skins")).unwrap();
        std::fs::write(cache_path(&dir, "short"), [1, 2, 3]).unwrap();

        assert_eq!(load_cached(&dir, "short"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_cache_entry_round_trips() {
        let dir = std::env::temp_dir().join(format!("erkuia-avatar-ok-{}", std::process::id()));
        let head = vec![7_u8; HEAD_BYTES];

        store(&dir, "abc", &head).unwrap();

        assert_eq!(load_cached(&dir, "abc"), Some(head));

        std::fs::remove_dir_all(&dir).ok();
    }
}
