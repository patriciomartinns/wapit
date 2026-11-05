use std::collections::HashMap;

use base64::Engine;
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

const EDITABLE: &[&str] = &["APPROVED", "REJECTED", "PAUSED"];
const MAX_BULK_DELETE: usize = 100;

/// Applies a `POST /{template_id}` edit, which may change the components and the category.
/// Like Meta, only approved, rejected or paused templates can be edited, and an approved one keeps its category.
fn apply_edit(template: &mut Template, input: &Value) -> Result<(), GraphError> {
    if input.get("components").is_none() && input.get("category").is_none() {
        return Err(invalid(
            "Provide components or category to edit the template",
        ));
    }

    let status = template.status.to_uppercase();

    if !EDITABLE.contains(&status.as_str()) {
        return Err(invalid(format!(
            "Only templates with an APPROVED, REJECTED or PAUSED status can be edited, this one is {status}"
        )));
    }

    let category = input.get("category").map(|_| category(input)).transpose()?;

    if let Some(category) = &category
        && status == "APPROVED"
        && template
            .category
            .as_deref()
            .is_none_or(|current| !current.eq_ignore_ascii_case(category))
    {
        return Err(invalid(
            "You cannot edit the category of an approved template.",
        ));
    }

    if input.get("components").is_some() {
        template.components = components(input)?;
    }

    if category.is_some() {
        template.category = category;
    }

    Ok(())
}

/// The filters and paging of a `GET /{waba_id}/message_templates` request.
#[derive(Debug, Default)]
pub struct ListQuery {
    name: Option<String>,
    statuses: Vec<String>,
    categories: Vec<String>,
    languages: Vec<String>,
    limit: Option<usize>,
    after: Option<usize>,
    before: Option<usize>,
}

/// Graph API list parameters come as one value, a comma separated list or a JSON array.
fn list_parameter(raw: Option<&String>) -> Vec<String> {
    let Some(raw) = raw.map(|raw| raw.trim()).filter(|raw| !raw.is_empty()) else {
        return Vec::new();
    };

    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_else(|_| raw.split(',').map(str::to_owned).collect())
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn encode_cursor(position: usize) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(position.to_string())
}

fn decode_cursor(field: &str, cursor: &str) -> Result<usize, GraphError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| invalid(format!("{field}: {cursor} is not a valid cursor")))
}

impl ListQuery {
    pub fn parse(query: &HashMap<String, String>) -> Result<Self, GraphError> {
        let limit = query
            .get("limit")
            .map(|limit| {
                limit
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| invalid(format!("limit: {limit} is not a positive number")))
            })
            .transpose()?;

        Ok(Self {
            name: query.get("name").filter(|name| !name.is_empty()).cloned(),
            statuses: list_parameter(query.get("status")),
            categories: list_parameter(query.get("category")),
            languages: list_parameter(query.get("language")),
            limit,
            after: query
                .get("after")
                .map(|cursor| decode_cursor("after", cursor))
                .transpose()?,
            before: query
                .get("before")
                .map(|cursor| decode_cursor("before", cursor))
                .transpose()?,
        })
    }

    fn matches(&self, template: &Template) -> bool {
        let any = |wanted: &[String], value: Option<&str>| {
            wanted.is_empty()
                || value.is_some_and(|value| {
                    wanted
                        .iter()
                        .any(|wanted| wanted.eq_ignore_ascii_case(value))
                })
        };

        self.name.as_ref().is_none_or(|name| &template.name == name)
            && any(&self.statuses, Some(&template.status))
            && any(&self.categories, template.category.as_deref())
            && any(&self.languages, Some(&template.language))
    }

    fn page_link(&self, base_url: &str, cursor: (&str, usize), limit: usize) -> String {
        let mut parameters = vec![format!("limit={limit}")];

        for (key, values) in [
            ("status", &self.statuses),
            ("category", &self.categories),
            ("language", &self.languages),
        ] {
            if !values.is_empty() {
                parameters.push(format!("{key}={}", values.join(",")));
            }
        }

        if let Some(name) = &self.name {
            parameters.push(format!("name={name}"));
        }

        parameters.push(format!("{}={}", cursor.0, encode_cursor(cursor.1)));

        format!("{base_url}?{}", parameters.join("&"))
    }
}

/// The app's templates in file order, as `GET /{waba_id}/message_templates` lists them: filtered, and paged when
/// the request sets `limit`. `base_url` is the endpoint the `next` and `previous` links point to.
pub fn list(config: &ConfigStore, app_id: &str, query: &ListQuery, base_url: &str) -> Value {
    let matching: Vec<Template> = config
        .templates_for(app_id)
        .iter()
        .filter(|template| query.matches(template))
        .cloned()
        .collect();
    let total = matching.len();
    let limit = query.limit.unwrap_or(total.max(1));
    let (start, end) = match (query.after, query.before) {
        (Some(after), _) => (after.min(total), (after + limit).min(total)),
        (None, Some(before)) => (before.min(total).saturating_sub(limit), before.min(total)),
        (None, None) => (0, limit.min(total)),
    };
    let mut paging =
        json!({ "cursors": { "before": encode_cursor(start), "after": encode_cursor(end) } });

    if end < total {
        paging["next"] = json!(query.page_link(base_url, ("after", end), limit));
    }

    if start > 0 {
        paging["previous"] = json!(query.page_link(base_url, ("before", start), limit));
    }

    json!({ "data": &matching[start..end], "paging": paging })
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

/// Deletes every language of the named template, only the one whose id is `hsm_id`, or every id in `hsm_ids`.
/// A bulk delete is all or nothing: one unknown id and nothing is deleted. Disabled templates cannot be deleted.
pub fn delete<S: std::hash::BuildHasher>(
    config: &ConfigStore,
    app_id: &str,
    query: &HashMap<String, String, S>,
) -> Result<(), GraphError> {
    let name = query.get("name").filter(|name| !name.is_empty());
    let hsm_id = query.get("hsm_id").filter(|id| !id.is_empty());
    let hsm_ids = list_parameter(query.get("hsm_ids"));

    if !hsm_ids.is_empty() {
        if name.is_some() || hsm_id.is_some() {
            return Err(invalid("hsm_ids cannot be combined with name or hsm_id"));
        }

        if hsm_ids.len() > MAX_BULK_DELETE {
            return Err(invalid(format!(
                "hsm_ids: at most {MAX_BULK_DELETE} template ids per request, received {}",
                hsm_ids.len()
            )));
        }
    }

    config.edit_templates(app_id, |all| {
        let doomed: Vec<usize> = if hsm_ids.is_empty() {
            let name = name.ok_or_else(|| GraphError::required("name"))?;
            let doomed: Vec<usize> = all
                .iter()
                .enumerate()
                .filter(|(_, template)| {
                    &template.name == name
                        && hsm_id.is_none_or(|id| template.id.as_ref() == Some(id))
                })
                .map(|(index, _)| index)
                .collect();

            if doomed.is_empty() {
                return Err(GraphError::invalid_parameter(format!(
                    "Message template \"{name}\" was not found"
                )));
            }

            doomed
        } else {
            hsm_ids
                .iter()
                .map(|id| {
                    all.iter()
                        .position(|template| template.id.as_ref() == Some(id))
                        .ok_or_else(|| {
                            GraphError::invalid_parameter(format!(
                                "hsm_ids: template {id} was not found, nothing was deleted"
                            ))
                        })
                })
                .collect::<Result<_, _>>()?
        };

        if let Some(disabled) = doomed
            .iter()
            .map(|index| &all[*index])
            .find(|template| template.status.eq_ignore_ascii_case("DISABLED"))
        {
            return Err(invalid(format!(
                "Message template \"{}\" is disabled and cannot be deleted",
                disabled.name
            )));
        }

        let mut index = 0;
        all.retain(|_| {
            let keep = !doomed.contains(&index);
            index += 1;
            keep
        });

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
