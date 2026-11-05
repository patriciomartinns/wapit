use rand::RngExt;
use serde_json::Value;

use serde_json::json;

use crate::config::{ConfigStore, Template, TemplateComponent};
use crate::graph_error::GraphError;

const CATEGORIES: &[&str] = &["AUTHENTICATION", "MARKETING", "UTILITY"];
const BUTTON_TYPES: &[&str] = &[
    "QUICK_REPLY",
    "URL",
    "PHONE_NUMBER",
    "COPY_CODE",
    "OTP",
    "FLOW",
    "CATALOG",
    "MPM",
];

fn invalid(details: impl Into<String>) -> GraphError {
    GraphError::invalid_parameter(details)
}

fn category(input: &Value) -> Result<String, GraphError> {
    let category = input
        .get("category")
        .and_then(Value::as_str)
        .ok_or_else(|| GraphError::required("category"))?
        .to_uppercase();

    if CATEGORIES.contains(&category.as_str()) {
        Ok(category)
    } else {
        Err(invalid(format!(
            "category: {category} is not one of {}",
            CATEGORIES.join(", ")
        )))
    }
}

fn components(input: &Value) -> Result<Vec<TemplateComponent>, GraphError> {
    let raw = input
        .get("components")
        .cloned()
        .ok_or_else(|| GraphError::required("components"))?;
    let components: Vec<TemplateComponent> =
        serde_json::from_value(raw).map_err(|error| invalid(format!("components: {error}")))?;

    let bodies: Vec<&TemplateComponent> = components
        .iter()
        .filter(|component| component.kind.eq_ignore_ascii_case("BODY"))
        .collect();

    match bodies.as_slice() {
        [body]
            if body
                .text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= 1024) => {}
        [_] => {
            return Err(invalid(
                "components: the BODY text is required and must be at most 1024 characters",
            ));
        }
        _ => {
            return Err(invalid(
                "components: a template needs exactly one BODY component",
            ));
        }
    }

    for component in &components {
        match component.kind.to_uppercase().as_str() {
            "HEADER"
                if component
                    .format
                    .as_deref()
                    .unwrap_or("TEXT")
                    .eq_ignore_ascii_case("TEXT") =>
            {
                if component
                    .text
                    .as_deref()
                    .is_none_or(|text| text.trim().is_empty() || text.chars().count() > 60)
                {
                    return Err(invalid(
                        "components: a TEXT HEADER needs text of at most 60 characters",
                    ));
                }
            }
            "FOOTER"
                if component
                    .text
                    .as_deref()
                    .is_none_or(|text| text.chars().count() > 60) =>
            {
                return Err(invalid(
                    "components: the FOOTER text must be at most 60 characters",
                ));
            }
            "BUTTONS" => {
                let buttons = component.buttons.as_deref().unwrap_or_default();

                if buttons.is_empty() || buttons.len() > 10 {
                    return Err(invalid(
                        "components: BUTTONS needs between 1 and 10 buttons",
                    ));
                }

                if let Some(button) = buttons
                    .iter()
                    .find(|button| !BUTTON_TYPES.contains(&button.kind.to_uppercase().as_str()))
                {
                    return Err(invalid(format!(
                        "components: button type {} is not supported",
                        button.kind
                    )));
                }
            }
            "HEADER" | "BODY" | "FOOTER" => {}
            other => {
                return Err(invalid(format!(
                    "components: component type {other} is not supported"
                )));
            }
        }
    }

    Ok(components)
}

/// Builds a template from a `POST /{waba_id}/message_templates` body. Local templates are approved right away,
/// so the app can send them without a review step.
fn build(existing: &[Template], input: &Value) -> Result<Template, GraphError> {
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| GraphError::required("name"))?;

    if name.is_empty()
        || name.len() > 512
        || !name
            .chars()
            .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_')
    {
        return Err(invalid(
            "name: use 1 to 512 lowercase letters, digits and underscores",
        ));
    }

    let language = input
        .get("language")
        .and_then(Value::as_str)
        .filter(|language| !language.is_empty())
        .ok_or_else(|| GraphError::required("language"))?;

    if existing
        .iter()
        .any(|template| template.name == name && template.language == language)
    {
        return Err(GraphError::new(100, "(#100) Invalid parameter")
            .subcode(2_388_024)
            .details(format!(
                "Message template \"{name}\" already has content in {language}"
            )));
    }

    let id: u64 = rand::rng().random_range(1_000_000_000_000_000..10_000_000_000_000_000);

    Ok(Template {
        name: name.to_owned(),
        language: language.to_owned(),
        status: "APPROVED".into(),
        category: Some(category(input)?),
        parameter_format: input
            .get("parameter_format")
            .and_then(Value::as_str)
            .map(str::to_uppercase),
        components: components(input)?,
        id: Some(id.to_string()),
        extra: input
            .as_object()
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "name"
                                | "language"
                                | "category"
                                | "parameter_format"
                                | "components"
                                | "id"
                                | "status"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Applies a `POST /{template_id}` edit, which may change the components and the category.
fn apply_edit(template: &mut Template, input: &Value) -> Result<(), GraphError> {
    if input.get("components").is_none() && input.get("category").is_none() {
        return Err(invalid(
            "Provide components or category to edit the template",
        ));
    }

    if input.get("components").is_some() {
        template.components = components(input)?;
    }

    if input.get("category").is_some() {
        template.category = Some(category(input)?);
    }

    Ok(())
}

/// The app's templates in file order, as `GET /{waba_id}/message_templates` lists them.
pub fn list(config: &ConfigStore, app_id: &str, name: Option<&str>) -> Vec<Template> {
    config
        .templates_for(app_id)
        .iter()
        .filter(|template| name.is_none_or(|name| template.name == name))
        .cloned()
        .collect()
}

/// Creates a template at the end of the file and answers like Meta: its id, status and category.
pub fn create(config: &ConfigStore, app_id: &str, input: &Value) -> Result<Value, GraphError> {
    config.edit_templates(app_id, |all| {
        let template = build(all, input)?;
        let response =
            json!({ "id": template.id, "status": template.status, "category": template.category });
        all.push(template);
        Ok(response)
    })
}

pub fn edit(
    config: &ConfigStore,
    app_id: &str,
    template_id: &str,
    input: &Value,
) -> Result<(), GraphError> {
    config.edit_templates(app_id, |all| {
        let template = all
            .iter_mut()
            .find(|template| template.id.as_deref() == Some(template_id))
            .ok_or_else(|| GraphError::unsupported("POST", template_id))?;
        apply_edit(template, input)
    })
}

/// Deletes every language of the named template, or only the one whose id is `hsm_id`.
pub fn delete(
    config: &ConfigStore,
    app_id: &str,
    name: Option<&str>,
    hsm_id: Option<&str>,
) -> Result<(), GraphError> {
    let name = name
        .filter(|name| !name.is_empty())
        .ok_or_else(|| GraphError::required("name"))?;

    config.edit_templates(app_id, |all| {
        let before = all.len();
        all.retain(|template| {
            !(template.name == name && hsm_id.is_none_or(|id| template.id.as_deref() == Some(id)))
        });

        if all.len() == before {
            return Err(GraphError::invalid_parameter(format!(
                "Message template \"{name}\" was not found"
            )));
        }

        Ok(())
    })
}

/// Replaces the app's templates with the ones `wapit sync` read from Meta.
pub fn replace_all(
    config: &ConfigStore,
    app_id: &str,
    templates: Vec<Template>,
) -> Result<(), GraphError> {
    config.edit_templates(app_id, |all| {
        *all = templates;
        Ok(())
    })
}
