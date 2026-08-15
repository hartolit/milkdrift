use super::support::*;

#[test]
fn configuration_declaration_must_match_required_primary() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::MixedF16F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_unsupported(loader.inspect(&source));

    let f32_fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    for declaration in [ConfigDeclaration::F16, ConfigDeclaration::Bf16] {
        let source = f32_fixture.source(declaration)?;
        assert_unsupported(loader.inspect(&source));
    }
    Ok(())
}

#[test]
fn mixed_required_layouts_require_matching_configuration_declarations() -> TestResult {
    for (profile, declaration) in [
        (RequiredProfile::MixedF16F32, ConfigDeclaration::F16),
        (RequiredProfile::MixedBf16F32, ConfigDeclaration::Bf16),
    ] {
        let fixture = TinyLlamaFixture::create(profile, &[])?;
        let absent = fixture.source(ConfigDeclaration::Absent)?;
        let mut loader = CandleLlamaLoader::new(BACKEND);
        assert_unsupported(loader.inspect(&absent));
        assert_unsupported(loader.prepare_load(&absent, &load_configuration()));

        let declared = fixture.source(declaration)?;
        loader.inspect(&declared).map_err(debug_error)?;
        let prepared = loader
            .prepare_load(&declared, &load_configuration())
            .map_err(debug_error)?;
        drop(prepared);
    }
    Ok(())
}

#[test]
fn unsupported_and_conflicting_config_declarations_are_rejected() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let loader = CandleLlamaLoader::new(BACKEND);
    for declaration in [ConfigDeclaration::Unsupported, ConfigDeclaration::Conflict] {
        let source = fixture.source(declaration)?;
        let error = loader
            .inspect(&source)
            .err()
            .ok_or_else(|| "unsupported configuration was admitted".to_owned())?;
        let LoadError::Backend(failure) = error else {
            return Err(format!("unexpected configuration failure: {error:?}"));
        };
        let context = failure
            .context
            .ok_or_else(|| "configuration failure lost its stage".to_owned())?;
        assert_eq!(context.stage, LoadFailureStage::CompatibilityValidation);
        assert_eq!(context.tensor, None);
    }
    Ok(())
}
