//! Romanised Hindi / Hinglish protected tokens.
//!
//! Every other dictation stack treats these as noise, which means a dropped
//! `nahi` or `mat` silently inverts the instruction. Spelling is not
//! standardised in romanised Hindi, so common variants are listed explicitly.

pub const NEGATION: &[&str] = &[
    "nahi", "nahin", "nai", "nhi", "na", "naa", "mat", "mt", "bina", "binaa", "koi", "kabhi",
    "bilkul", "kuchh", "kuch",
];

pub const TEMPORAL: &[&str] = &[
    "pehle", "pehla", "baad", "abhi", "kal", "aaj", "parso", "phir", "tab", "jab", "turant",
    "jaldi", "der", "raat", "subah", "shaam", "dopahar",
];

pub const DIRECTION: &[&str] = &[
    "karo", "mat", "band", "chalu", "hatao", "jodo", "badhao", "ghatao", "bhejo", "roko", "chalao",
    "kholo", "banao", "mitao", "daalo", "nikalo", "rakho", "lao",
];

pub const QUANTIFIER: &[&str] = &[
    "sab", "sabhi", "sirf", "bas", "poora", "pura", "aadha", "adha", "dono", "har", "thoda",
    "zyada", "kam",
];

pub const MODALITY: &[&str] = &[
    "chahiye", "zaroori", "zaruri", "shayad", "mumkin", "sakta", "sakte", "sakti", "padega",
    "padegi", "hoga", "hogi", "zarur", "zaroor",
];

pub const NUMBER_WORDS: &[&str] = &[
    "ek", "do", "teen", "char", "paanch", "panch", "chhe", "che", "saat", "aath", "nau", "das",
    "gyarah", "barah", "bees", "pachas", "sau", "hazaar", "hazar", "lakh", "crore", "karod",
];
