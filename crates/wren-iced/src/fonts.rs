use iced::Font;

pub const UI: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font {
    weight: iced::font::Weight::Medium,
    ..UI
};
pub const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..UI
};
pub const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..UI
};
pub const EDITOR: Font = Font::with_name("JetBrains Mono");

pub const FACES: &[&[u8]] = &[
    include_bytes!("../../../third_party/inter/Inter-Regular.ttf"),
    include_bytes!("../../../third_party/inter/Inter-Medium.ttf"),
    include_bytes!("../../../third_party/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../../third_party/inter/Inter-Bold.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
];

pub const LICENSES: &str = concat!(
    "Inter\n\n",
    include_str!("../../../third_party/inter/LICENSE.txt"),
    "\n\nJetBrains Mono\n\n",
    include_str!("../../../third_party/jetbrains-mono/OFL.txt"),
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_and_redistribution_notices_are_present() {
        assert_eq!(FACES.len(), 8);
        for face in FACES {
            assert!(face.len() > 1000);
            assert_eq!(&face[..4], &[0, 1, 0, 0]);
        }
        assert!(LICENSES.contains("The Inter Project Authors"));
        assert!(LICENSES.contains("The JetBrains Mono Project Authors"));
        assert_eq!(
            LICENSES
                .matches("SIL OPEN FONT LICENSE Version 1.1")
                .count(),
            2
        );
    }
}
