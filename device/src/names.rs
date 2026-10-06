//! Human-friendly device names.
//!
//! On first boot a device draws an adjective and a noun from these lists and
//! keeps the result ("Brave Otter"). The name is persisted with the identity,
//! advertised as the mDNS service instance so Home Assistant discovers it under
//! that name, and shown on the pairing screen so the panel and the discovery
//! list can be matched up.

use anyhow::Result;

use crate::random;

/// Adjectives used to baptise a device.
const ADJECTIVES: &[&str] = &[
    "Amber",
    "Bold",
    "Brave",
    "Bright",
    "Brisk",
    "Calm",
    "Clever",
    "Copper",
    "Coral",
    "Cozy",
    "Crisp",
    "Curious",
    "Dapper",
    "Daring",
    "Dusky",
    "Eager",
    "Fancy",
    "Fleet",
    "Foggy",
    "Gentle",
    "Gilded",
    "Glad",
    "Golden",
    "Graceful",
    "Happy",
    "Hardy",
    "Hazy",
    "Humble",
    "Ivory",
    "Jade",
    "Jolly",
    "Keen",
    "Lively",
    "Lucky",
    "Lunar",
    "Mellow",
    "Merry",
    "Misty",
    "Nimble",
    "Noble",
    "Olive",
    "Opal",
    "Patient",
    "Playful",
    "Plucky",
    "Quiet",
    "Rapid",
    "Royal",
    "Rustic",
    "Sandy",
    "Scarlet",
    "Silent",
    "Silver",
    "Sleepy",
    "Snowy",
    "Solar",
    "Spry",
    "Steady",
    "Sunny",
    "Swift",
    "Tidy",
    "Velvet",
    "Vivid",
    "Wandering",
    "Wild",
    "Winter",
    "Witty",
];

/// Nouns (mostly animals) used to baptise a device.
const NOUNS: &[&str] = &[
    "Badger",
    "Beaver",
    "Bison",
    "Bobcat",
    "Condor",
    "Crane",
    "Dolphin",
    "Eagle",
    "Elk",
    "Falcon",
    "Ferret",
    "Finch",
    "Fox",
    "Gecko",
    "Hare",
    "Heron",
    "Ibex",
    "Ibis",
    "Jay",
    "Kestrel",
    "Koala",
    "Lemur",
    "Lizard",
    "Lynx",
    "Magpie",
    "Mink",
    "Moose",
    "Narwhal",
    "Newt",
    "Ocelot",
    "Osprey",
    "Otter",
    "Owl",
    "Panda",
    "Panther",
    "Parrot",
    "Pelican",
    "Penguin",
    "Puma",
    "Puffin",
    "Quail",
    "Rabbit",
    "Raccoon",
    "Raven",
    "Robin",
    "Salamander",
    "Seal",
    "Shark",
    "Sparrow",
    "Squirrel",
    "Stag",
    "Starling",
    "Swan",
    "Tapir",
    "Tortoise",
    "Toucan",
    "Trout",
    "Turtle",
    "Viper",
    "Vole",
    "Walrus",
    "Wombat",
    "Wren",
    "Yak",
    "Zebra",
];

/// Draw a fresh `<Adjective> <Noun>` name.
pub fn generate() -> Result<String> {
    let adjective = pick(ADJECTIVES)?;
    let noun = pick(NOUNS)?;
    Ok(format!("{adjective} {noun}"))
}

fn pick(words: &[&'static str]) -> Result<&'static str> {
    let index = random::u32_below(words.len() as u32)? as usize;
    Ok(words[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_one_adjective_and_one_noun() {
        let name = generate().unwrap();
        let mut parts = name.split(' ');
        let adjective = parts.next().unwrap();
        let noun = parts.next().unwrap();
        assert!(parts.next().is_none(), "unexpected extra word in {name:?}");
        assert!(
            ADJECTIVES.contains(&adjective),
            "unknown adjective {adjective:?}"
        );
        assert!(NOUNS.contains(&noun), "unknown noun {noun:?}");
    }

    #[test]
    fn word_lists_are_disjoint_and_large_enough() {
        assert!(ADJECTIVES.len() >= 32, "adjective list is too small");
        assert!(NOUNS.len() >= 32, "noun list is too small");
        for adjective in ADJECTIVES {
            assert!(!NOUNS.contains(adjective), "{adjective} is in both lists");
        }
    }
}
