//! Command implementations.

use crate::cli::{CheckArgs, ConfigArgs, DiffArgs, LexiconCommand};
use crate::input::{build_config, build_context, load_cleaned, load_transcript};
use crate::render::{Style, exit_code, print_flag, print_verdict};
use anyhow::{Result, bail};
use readback_core::{CheckInput, Lexicon, Readback, SemanticClass, check_cleanup};

const ALL_CLASSES: [SemanticClass; 8] = [
    SemanticClass::Negation,
    SemanticClass::Direction,
    SemanticClass::Environment,
    SemanticClass::Number,
    SemanticClass::Temporal,
    SemanticClass::Quantifier,
    SemanticClass::Modality,
    SemanticClass::ProtectedTerm,
];

fn class_name(class: SemanticClass) -> &'static str {
    match class {
        SemanticClass::Negation => "negation",
        SemanticClass::Direction => "direction",
        SemanticClass::Environment => "environment",
        SemanticClass::Number => "number",
        SemanticClass::Temporal => "temporal",
        SemanticClass::Quantifier => "quantifier",
        SemanticClass::Modality => "modality",
        SemanticClass::ProtectedTerm => "protected-term",
    }
}

fn parse_class(name: &str) -> Result<SemanticClass> {
    let wanted = name.to_lowercase().replace('_', "-");
    ALL_CLASSES
        .into_iter()
        .find(|c| class_name(*c) == wanted)
        .ok_or_else(|| {
            let names: Vec<&str> = ALL_CLASSES.into_iter().map(class_name).collect();
            anyhow::anyhow!(
                "unknown class {name:?}; expected one of: {}",
                names.join(", ")
            )
        })
}

fn lexicon_from(args: &ConfigArgs) -> Result<Lexicon> {
    let config = build_config(args)?;
    let mut lexicon = Lexicon::new(&config.locales);
    lexicon.protect(&config.vocabulary);
    Ok(lexicon)
}

pub fn check(args: CheckArgs, style: Style) -> Result<i32> {
    let loaded = load_transcript(&args.input)?;
    let cleaned = load_cleaned(&args.input)?;
    let config = build_config(&args.config)?;

    let mut input = CheckInput::new(loaded.transcript).with_context(build_context(&args.config));
    input.cleaned = cleaned;

    let verdict = Readback::with_config(config).check(input);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&verdict)?);
    } else if !args.quiet {
        print_verdict(&verdict, loaded.engine.label(), style);
    }

    Ok(if args.exit_zero {
        0
    } else {
        exit_code(verdict.action)
    })
}

pub fn explain(args: CheckArgs, style: Style) -> Result<i32> {
    let loaded = load_transcript(&args.input)?;
    let cleaned = load_cleaned(&args.input)?;
    let config = build_config(&args.config)?;
    let policy = config.policy.for_app(args.config.app.as_deref());
    let floor = config.risk.floor;

    let mut input = CheckInput::new(loaded.transcript).with_context(build_context(&args.config));
    input.cleaned = cleaned;
    let verdict = Readback::with_config(config).check(input);

    let semantic = verdict
        .flags
        .iter()
        .filter(|f| f.kind.is_semantic())
        .map(|f| f.severity.weight())
        .fold(0.0_f32, f32::max);
    let evidence = verdict.suspicion.max(semantic);
    let scale = floor + (1.0 - floor) * verdict.stakes;

    println!();
    println!("  {}", style.bold("evidence"));
    println!(
        "    suspicion              {:.3}   {}",
        verdict.suspicion,
        style.dim("how shaky the recognition looked")
    );
    println!(
        "    semantic severity      {semantic:.3}   {}",
        style.dim("worst meaning-changing edit")
    );
    println!("    max(evidence)          {evidence:.3}");
    println!();
    println!("  {}", style.bold("stakes"));
    println!(
        "    stakes                 {:.3}   {}",
        verdict.stakes,
        style.dim(&format!("scorer: {}", verdict.provenance.scorer))
    );
    println!(
        "    floor                  {floor:.3}   {}",
        style.dim("risk that applies regardless of stakes")
    );
    println!("    scale = floor + (1-floor)*stakes");
    println!("                           {scale:.3}");
    println!();
    println!("  {}", style.bold("risk"));
    println!("    risk = max(evidence) * scale");
    println!("                           {:.3}", verdict.risk);
    println!();
    println!("  {}", style.bold("policy"));
    println!(
        "    app                    {}",
        args.config
            .app
            .as_deref()
            .unwrap_or("(none, using default)")
    );
    println!("    highlight at           {:.3}", policy.highlight);
    println!("    hold at                {:.3}", policy.hold);
    println!("    decision               {:?}", verdict.action);
    println!();

    if !verdict.flags.is_empty() {
        println!("  {}", style.bold("flags"));
        for flag in &verdict.flags {
            print_flag(flag, &verdict.text, style);
        }
        println!();
    }

    Ok(exit_code(verdict.action))
}

pub fn diff(args: DiffArgs, style: Style) -> Result<i32> {
    let loaded = load_transcript(&args.input)?;
    let Some(cleaned) = load_cleaned(&args.input)? else {
        bail!("diff needs the polished rewrite: pass --cleaned <FILE> or --cleaned-text <STRING>");
    };
    let lexicon = lexicon_from(&args.config)?;
    let raw = loaded.transcript.primary.text;
    let outcome = check_cleanup(&raw, &cleaned, &lexicon);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "raw": raw,
                "cleaned": cleaned,
                "text": outcome.text,
                "reverted": outcome.reverted,
                "flags": outcome.flags,
            }))?
        );
        return Ok(i32::from(outcome.reverted));
    }

    println!();
    println!("  {}      {}", style.dim("raw"), raw);
    println!("  {}  {}", style.dim("cleaned"), cleaned);
    println!(
        "  {}    {}",
        style.dim("final"),
        crate::render::mark_text(&outcome.text, &outcome.flags, style)
    );
    println!();
    if outcome.reverted {
        println!("  {}", style.bold("the guard reverted:"));
        for flag in &outcome.flags {
            print_flag(flag, &outcome.text, style);
        }
    } else {
        println!(
            "  {}",
            style.dim("nothing protected was touched; the polish was kept")
        );
    }
    println!();

    Ok(i32::from(outcome.reverted))
}

pub fn lexicon(command: LexiconCommand, style: Style) -> Result<i32> {
    match command {
        LexiconCommand::Classify { words, config } => {
            let lexicon = lexicon_from(&config)?;
            println!();
            for word in &words {
                match lexicon.classify_str(word) {
                    Some(class) => println!(
                        "  {:<20} {:<16} {}",
                        style.bold(word),
                        class_name(class),
                        style.dim(&format!("stakes weight {:.2}", class.stakes_weight()))
                    ),
                    None => println!("  {:<20} {}", style.bold(word), style.dim("unprotected")),
                }
            }
            println!();
            Ok(0)
        }
        LexiconCommand::List { class, config } => {
            let lexicon = lexicon_from(&config)?;
            let classes = match class {
                Some(name) => vec![parse_class(&name)?],
                None => ALL_CLASSES.to_vec(),
            };
            println!();
            for class in classes {
                let words = lexicon.words(class);
                println!(
                    "  {} {}",
                    style.bold(class_name(class)),
                    style.dim(&format!("({} words)", words.len()))
                );
                if words.is_empty() {
                    println!("    {}", style.dim("(none loaded)"));
                } else {
                    println!("    {}", words.join(" "));
                }
                println!();
            }
            let destructive = lexicon.words_destructive();
            println!(
                "  {} {}",
                style.bold("destructive"),
                style.dim(&format!(
                    "({} words, raise stakes but are not protected)",
                    destructive.len()
                ))
            );
            println!("    {}", destructive.join(" "));
            println!();
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_names_round_trip() {
        for class in ALL_CLASSES {
            assert_eq!(parse_class(class_name(class)).unwrap(), class);
        }
    }

    #[test]
    fn underscores_are_accepted() {
        assert_eq!(
            parse_class("protected_term").unwrap(),
            SemanticClass::ProtectedTerm
        );
    }

    #[test]
    fn an_unknown_class_lists_the_valid_ones() {
        let err = parse_class("vibes").unwrap_err().to_string();
        assert!(err.contains("negation"), "{err}");
    }
}
