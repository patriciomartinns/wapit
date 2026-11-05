use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::config::{Template, TemplateButton};
use crate::graph_error::GraphError;
use crate::model::{BodyPart, Button, Issue, MediaRef, Rendered, TemplateRef, digits};

/// Meta treats a send without `type` as a text message.
pub const DEFAULT_MESSAGE_TYPE: &str = "text";

pub const SERVICE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

const MEDIA_TYPES: &[&str] = &["image", "document", "audio", "video", "sticker"];
const MESSAGE_TYPES: &[&str] = &[
    "text",
    "template",
    "interactive",
    "reaction",
    "location",
    "contacts",
    "image",
    "document",
    "audio",
    "video",
    "sticker",
];

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_]+)\s*\}\}").expect("valid regex"));
static BODY_PARAMETER_SPACING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\n\t]|\s{5,}").expect("valid regex"));

type Json = Map<String, Value>;

pub struct Outbound {
    pub kind: String,
    pub contact: String,
    pub input: String,
    pub rendered: Rendered,
    pub issues: Vec<Issue>,
    pub context_id: Option<String>,
    pub outside_window: bool,
}

pub struct OutboundContext<'a> {
    pub templates: &'a [Template],
    pub last_inbound_at: Option<i64>,
    pub now: i64,
}

fn object(value: Option<&Value>) -> Option<&Json> {
    value.and_then(Value::as_object)
}

fn string(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn objects(value: Option<&Value>) -> Vec<&Json> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn require_length(value: Option<&Value>, field: &str, max: usize) -> Result<String, GraphError> {
    let text = string(value)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| GraphError::required(field))?;

    if text.chars().count() > max {
        return Err(GraphError::invalid(format!(
            "{field} length invalid. Min length: 1, Max length: {max}"
        )));
    }

    Ok(text.to_owned())
}

pub fn placeholders(text: Option<&str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    for captures in PLACEHOLDER.captures_iter(text.unwrap_or_default()) {
        let name = captures[1].to_owned();

        if !found.contains(&name) {
            found.push(name);
        }
    }

    found
}

fn substitute(text: &str, values: &HashMap<String, String>) -> String {
    PLACEHOLDER
        .replace_all(text, |captures: &regex::Captures<'_>| {
            values
                .get(&captures[1])
                .cloned()
                .unwrap_or_else(|| captures[0].to_owned())
        })
        .into_owned()
}

fn substitute_parts(text: &str, values: &HashMap<String, String>) -> Vec<BodyPart> {
    let mut parts = Vec::new();
    let mut last = 0;

    for captures in PLACEHOLDER.captures_iter(text) {
        let Some(value) = values.get(&captures[1]) else {
            continue;
        };
        let whole = captures.get(0).expect("group 0 always matches");

        if whole.start() > last {
            parts.push(BodyPart {
                text: text[last..whole.start()].to_owned(),
                variable: false,
            });
        }

        parts.push(BodyPart {
            text: value.clone(),
            variable: true,
        });
        last = whole.end();
    }

    if last < text.len() {
        parts.push(BodyPart {
            text: text[last..].to_owned(),
            variable: false,
        });
    }

    parts
}

fn parameter_text(parameter: &Json, location: &str) -> Result<String, GraphError> {
    let fallback = |key: &str| {
        object(parameter.get(key))
            .and_then(|value| string(value.get("fallback_value")))
            .unwrap_or_default()
            .to_owned()
    };

    match string(parameter.get("type")).unwrap_or("text") {
        "text" => string(parameter.get("text"))
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                GraphError::new(131_008, "(#131008) Required parameter is missing")
                    .details(format!("{location}: parameter text is required"))
            }),
        "currency" => Ok(fallback("currency")),
        "date_time" => Ok(fallback("date_time")),
        "payload" => Ok(string(parameter.get("payload"))
            .unwrap_or_default()
            .to_owned()),
        other => Ok(format!("[{other}]")),
    }
}

fn bind_parameters(
    text: Option<&str>,
    parameters: &[&Json],
    location: &str,
    named: bool,
    strict_body_text: bool,
) -> Result<HashMap<String, String>, GraphError> {
    let mut expected = placeholders(text);

    if !named {
        expected.sort_by_key(|name| name.parse::<u64>().unwrap_or(u64::MAX));
    }

    if expected.len() != parameters.len() {
        return Err(GraphError::count_mismatch(
            location,
            parameters.len(),
            expected.len(),
        ));
    }

    let mut values = HashMap::new();

    for (index, parameter) in parameters.iter().enumerate() {
        let value = parameter_text(parameter, location)?;

        if strict_body_text && BODY_PARAMETER_SPACING.is_match(&value) {
            return Err(GraphError::new(132_018, "(#132018) There was an issue with the parameters in your template.").details(
                format!("{location}: param text cannot have new-line/tab characters or more than 4 consecutive spaces"),
            ));
        }

        if named {
            let name = string(parameter.get("parameter_name"));

            match name {
                Some(name) if expected.iter().any(|candidate| candidate == name) => {
                    values.insert(name.to_owned(), value);
                }
                _ => {
                    return Err(GraphError::invalid(format!(
                        "{location}: parameter_name {} is not defined in the template",
                        name.unwrap_or("(missing)")
                    )));
                }
            }
        } else {
            values.insert(expected[index].clone(), value);
        }
    }

    Ok(values)
}

fn component_parameters(component: Option<&Json>) -> Vec<&Json> {
    objects(component.and_then(|component| component.get("parameters")))
}

fn component_type(component: &Json) -> String {
    string(component.get("type"))
        .unwrap_or_default()
        .to_lowercase()
}

fn render_template(
    payload: &Json,
    templates: &[Template],
    issues: &mut Vec<Issue>,
) -> Result<Rendered, GraphError> {
    let name = string(payload.get("name"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| GraphError::required("template['name']"))?;
    let language = object(payload.get("language"))
        .and_then(|language| string(language.get("code")))
        .filter(|code| !code.is_empty())
        .ok_or_else(|| GraphError::required("template['language']['code']"))?;
    let requested = objects(payload.get("components"));
    let template_ref = |category: Option<String>| TemplateRef {
        name: name.to_owned(),
        language: language.to_owned(),
        category,
    };

    if templates.is_empty() {
        issues.push(Issue::NoLocalTemplates);

        let mut lines = Vec::new();

        for component in requested
            .iter()
            .filter(|component| string(component.get("type")) == Some("body"))
        {
            for (index, parameter) in component_parameters(Some(component)).iter().enumerate() {
                lines.push(format!(
                    "{{{{{}}}}} = {}",
                    index + 1,
                    parameter_text(parameter, "body")?
                ));
            }
        }

        return Ok(Rendered {
            template: Some(template_ref(None)),
            body: Some(lines.join("\n")),
            ..Rendered::of_kind("template")
        });
    }

    let template = templates
        .iter()
        .find(|candidate| candidate.name == name && candidate.language == language)
        .ok_or_else(|| {
            GraphError::new(
                132_001,
                "(#132001) Template name does not exist in the translation",
            )
            .details(format!(
                "template name ({name}) does not exist in {language}"
            ))
        })?;

    match template.status.to_uppercase().as_str() {
        "APPROVED" => {}
        "PAUSED" => {
            return Err(GraphError::new(132_015, "(#132015) Template is Paused")
                .details(format!("template {name} is paused")));
        }
        "DISABLED" => {
            return Err(
                GraphError::new(132_016, "(#132016) Template has been disabled")
                    .details(format!("template {name} is disabled")),
            );
        }
        status => {
            return Err(GraphError::new(
                132_001,
                "(#132001) Template name does not exist in the translation",
            )
            .details(format!(
                "template name ({name}) does not exist in {language} (status {status})"
            )));
        }
    }

    let named = template
        .parameter_format
        .as_deref()
        .is_some_and(|format| format.eq_ignore_ascii_case("NAMED"));
    let by_type = |kind: &str| {
        template
            .components
            .iter()
            .find(|component| component.kind.eq_ignore_ascii_case(kind))
    };
    let requested_of = |kind: &str| {
        requested
            .iter()
            .copied()
            .find(|component| component_type(component) == kind)
    };
    let mut rendered = Rendered {
        template: Some(template_ref(template.category.clone())),
        ..Rendered::of_kind("template")
    };

    if let Some(header) = by_type("HEADER") {
        let format = header.format.as_deref().unwrap_or("TEXT").to_uppercase();
        let requested_header = requested_of("header");

        if format == "TEXT" {
            let values = bind_parameters(
                header.text.as_deref(),
                &component_parameters(requested_header),
                "header",
                named,
                false,
            )?;
            rendered.header = Some(substitute(
                header.text.as_deref().unwrap_or_default(),
                &values,
            ));
        } else if format != "LOCATION" {
            let parameter = component_parameters(requested_header).first().copied();
            let received = parameter
                .and_then(|parameter| string(parameter.get("type")))
                .map(str::to_uppercase);
            let media =
                parameter.and_then(|parameter| object(parameter.get(&format.to_lowercase())));

            let Some(media) = media.filter(|_| received.as_deref() == Some(format.as_str())) else {
                return Err(GraphError::new(
                    132_012,
                    "(#132012) Parameter format does not match format in the created template",
                )
                .details(format!(
                    "header: Format mismatch, expected {format}, received {}",
                    received.as_deref().unwrap_or("UNKNOWN")
                )));
            };

            rendered.media = Some(MediaRef {
                id: string(media.get("id")).map(str::to_owned),
                link: string(media.get("link")).map(str::to_owned),
                filename: string(media.get("filename")).map(str::to_owned),
                mime_type: Some(format.to_lowercase()),
            });
        }
    } else if let Some(header) = requested_of("header") {
        return Err(GraphError::count_mismatch(
            "header",
            component_parameters(Some(header)).len(),
            0,
        ));
    }

    let body = by_type("BODY");
    let body_text = body.and_then(|body| body.text.as_deref());
    let body_values = bind_parameters(
        body_text,
        &component_parameters(requested_of("body")),
        "body",
        named,
        true,
    )?;
    let substituted = substitute(body_text.unwrap_or_default(), &body_values);

    if substituted.chars().count() > 1024 {
        return Err(
            GraphError::new(132_005, "(#132005) Translated text too long")
                .details("body: text after variable substitution exceeds 1024 characters"),
        );
    }

    rendered.body_parts = Some(substitute_parts(
        body_text.unwrap_or_default(),
        &body_values,
    ));
    rendered.body = Some(substituted);
    rendered.footer = by_type("FOOTER").and_then(|footer| footer.text.clone());

    let buttons = by_type("BUTTONS")
        .and_then(|component| component.buttons.as_deref())
        .unwrap_or_default();
    let requested_buttons: Vec<&Json> = requested
        .iter()
        .copied()
        .filter(|component| component_type(component) == "button")
        .collect();
    rendered.buttons = Some(render_template_buttons(buttons, &requested_buttons)?);

    Ok(rendered)
}

fn button_index(component: &Json) -> Option<usize> {
    match component.get("index") {
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|index| usize::try_from(index).ok()),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn render_template_buttons(
    buttons: &[TemplateButton],
    requested: &[&Json],
) -> Result<Vec<Button>, GraphError> {
    let mut by_index: HashMap<usize, &Json> = HashMap::new();

    for component in requested {
        let index = button_index(component)
            .filter(|index| *index < buttons.len())
            .ok_or_else(|| {
                let shown = component.get("index").map_or_else(
                    || "undefined".to_owned(),
                    |value| value.to_string().replace('"', ""),
                );
                GraphError::invalid(format!(
                    "buttons: button index {shown} is out of range (template has {} buttons)",
                    buttons.len()
                ))
            })?;

        if by_index.insert(index, component).is_some() {
            return Err(GraphError::invalid(format!(
                "buttons: button index {index} was sent more than once"
            )));
        }
    }

    buttons
        .iter()
        .enumerate()
        .map(|(index, button)| {
            let kind = button.kind.to_uppercase();
            let component = by_index.get(&index).copied();
            let sub_type = component
                .and_then(|component| string(component.get("sub_type")))
                .map(str::to_lowercase);

            if component.is_some() && sub_type.as_deref() != Some(kind.to_lowercase().as_str()) {
                return Err(GraphError::invalid(format!(
                    "buttons: button at index {index} is {kind}, received sub_type {}",
                    sub_type.as_deref().unwrap_or("(missing)")
                )));
            }

            match kind.as_str() {
                "URL" => {
                    let url = button.url.as_deref().unwrap_or_default();
                    let location = format!("button at index {index} type Url");
                    let values = bind_parameters(
                        Some(url),
                        &component_parameters(component),
                        &location,
                        false,
                        false,
                    )?;

                    Ok(Button {
                        kind: "url".into(),
                        text: button.text.clone(),
                        url: Some(substitute(url, &values)),
                        id: None,
                        payload: None,
                    })
                }
                "QUICK_REPLY" => {
                    let payload = match component_parameters(component).first() {
                        Some(parameter) => {
                            parameter_text(parameter, &format!("button at index {index}"))?
                        }
                        None => button.text.clone(),
                    };

                    Ok(Button {
                        kind: "quick_reply".into(),
                        text: button.text.clone(),
                        payload: Some(payload),
                        id: None,
                        url: None,
                    })
                }
                "COPY_CODE" => {
                    let code = match component_parameters(component).first() {
                        Some(parameter) => Some(copy_code(parameter, index)?),
                        None => None,
                    };

                    Ok(Button {
                        kind: "copy_code".into(),
                        text: button.text.clone(),
                        payload: code,
                        id: None,
                        url: None,
                    })
                }
                _ => {
                    if let Some(component) = component {
                        let location = format!("button at index {index} type {kind}");
                        return Err(GraphError::count_mismatch(
                            &location,
                            component_parameters(Some(component)).len(),
                            0,
                        ));
                    }

                    Ok(Button {
                        kind: kind.to_lowercase(),
                        text: button.text.clone(),
                        url: button.url.clone().or_else(|| button.phone_number.clone()),
                        id: None,
                        payload: None,
                    })
                }
            }
        })
        .collect()
}

fn copy_code(parameter: &Json, index: usize) -> Result<String, GraphError> {
    string(parameter.get("coupon_code"))
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            GraphError::new(131_008, "(#131008) Required parameter is missing").details(format!(
                "button at index {index}: parameter coupon_code is required"
            ))
        })
}

fn render_interactive(interactive: &Json) -> Result<Rendered, GraphError> {
    let kind = string(interactive.get("type"));
    let body = object(interactive.get("body"));
    let header = object(interactive.get("header"));
    let footer = object(interactive.get("footer"));
    let empty = Json::new();
    let action = object(interactive.get("action")).unwrap_or(&empty);
    let body_text = body.and_then(|body| body.get("text"));

    let mut rendered = Rendered {
        header: header
            .and_then(|header| string(header.get("text")))
            .map(str::to_owned),
        footer: footer
            .map(|footer| require_length(footer.get("text"), "interactive['footer']['text']", 60))
            .transpose()?,
        ..Rendered::of_kind(format!("interactive_{}", kind.unwrap_or("unknown")))
    };

    if let Some(header) = header.filter(|header| string(header.get("type")) == Some("text")) {
        require_length(header.get("text"), "interactive['header']['text']", 60)?;
    }

    match kind {
        Some("button") => {
            rendered.body = Some(require_length(
                body_text,
                "interactive['body']['text']",
                1024,
            )?);
            let buttons: Vec<Option<&Json>> = action
                .get("buttons")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(Value::as_object).collect())
                .unwrap_or_default();

            if !(1..=3).contains(&buttons.len()) {
                return Err(GraphError::invalid(format!(
                    "interactive['action']['buttons']: must contain between 1 and 3 buttons, received {}",
                    buttons.len()
                )));
            }

            let mut ids = HashSet::new();
            let mut titles = HashSet::new();
            let mut rendered_buttons = Vec::new();

            for (index, button) in buttons.into_iter().enumerate() {
                let reply = button.and_then(|button| object(button.get("reply")));
                let id = require_length(
                    reply.and_then(|reply| reply.get("id")),
                    &format!("buttons[{index}]['reply']['id']"),
                    256,
                )?;
                let title = require_length(
                    reply.and_then(|reply| reply.get("title")),
                    &format!("buttons[{index}]['reply']['title']"),
                    20,
                )?;

                if !ids.insert(id.clone()) || !titles.insert(title.clone()) {
                    return Err(GraphError::invalid(format!(
                        "interactive['action']['buttons']: button ids and titles must be unique ({id} / {title})"
                    )));
                }

                rendered_buttons.push(Button {
                    kind: "reply".into(),
                    id: Some(id),
                    text: title,
                    url: None,
                    payload: None,
                });
            }

            rendered.buttons = Some(rendered_buttons);
        }
        Some("list") => {
            rendered.body = Some(require_length(
                body_text,
                "interactive['body']['text']",
                4096,
            )?);
            require_length(action.get("button"), "interactive['action']['button']", 20)?;

            let sections = objects(action.get("sections"));

            if sections.len() > 10 {
                return Err(GraphError::invalid(format!(
                    "interactive['action']['sections']: must contain at most 10 sections, received {}",
                    sections.len()
                )));
            }

            if sections.len() > 1 {
                for (index, section) in sections.iter().enumerate() {
                    require_length(
                        section.get("title"),
                        &format!("sections[{index}]['title']"),
                        24,
                    )?;
                }
            }

            let rows: Vec<&Json> = sections
                .iter()
                .flat_map(|section| objects(section.get("rows")))
                .collect();

            if !(1..=10).contains(&rows.len()) {
                return Err(GraphError::invalid(format!(
                    "interactive['action']['sections']: must contain between 1 and 10 rows, received {}",
                    rows.len()
                )));
            }

            let mut rendered_rows = Vec::new();

            for (index, row) in rows.iter().enumerate() {
                if let Some(description) = row.get("description") {
                    require_length(
                        Some(description),
                        &format!("rows[{index}]['description']"),
                        72,
                    )?;
                }

                rendered_rows.push(Button {
                    kind: "list_reply".into(),
                    id: Some(require_length(
                        row.get("id"),
                        &format!("rows[{index}]['id']"),
                        200,
                    )?),
                    text: require_length(row.get("title"), &format!("rows[{index}]['title']"), 24)?,
                    url: None,
                    payload: None,
                });
            }

            rendered.buttons = Some(rendered_rows);
        }
        Some("cta_url") => {
            let parameters = object(action.get("parameters"));
            rendered.body = Some(require_length(
                body_text,
                "interactive['body']['text']",
                1024,
            )?);
            rendered.buttons = Some(vec![Button {
                kind: "url".into(),
                text: require_length(
                    parameters.and_then(|parameters| parameters.get("display_text")),
                    "interactive['action']['parameters']['display_text']",
                    20,
                )?,
                url: Some(require_length(
                    parameters.and_then(|parameters| parameters.get("url")),
                    "interactive['action']['parameters']['url']",
                    2000,
                )?),
                id: None,
                payload: None,
            }]);
        }
        _ => {
            rendered.body = string(body_text).map(str::to_owned);
        }
    }

    Ok(rendered)
}

pub fn validate_outbound(
    payload: &Value,
    context: &OutboundContext<'_>,
) -> Result<Outbound, GraphError> {
    let empty = Json::new();
    let payload = payload.as_object().unwrap_or(&empty);

    if string(payload.get("messaging_product")) != Some("whatsapp") {
        return Err(GraphError::required("messaging_product"));
    }

    let to = string(payload.get("to"));
    let input = to
        .or_else(|| string(payload.get("recipient")))
        .filter(|input| !input.trim().is_empty())
        .ok_or_else(|| GraphError::required("to"))?;
    let contact = match to {
        Some(to) => digits(to),
        None => input.trim().to_owned(),
    };

    if let Some(to) = to
        && !(8..=15).contains(&contact.len())
    {
        return Err(GraphError::new(100, "(#100) Invalid parameter")
            .details(format!("to: {to} is not a valid WhatsApp phone number")));
    }

    let kind = string(payload.get("type")).unwrap_or(DEFAULT_MESSAGE_TYPE);

    if !MESSAGE_TYPES.contains(&kind) {
        return Err(GraphError::new(
            100,
            format!(
                "(#100) Param type must be one of {{{}}} - got \"{kind}\".",
                MESSAGE_TYPES.join(", ").to_uppercase()
            ),
        ));
    }

    let fields = object(payload.get(kind)).ok_or_else(|| GraphError::required(kind))?;
    let mut issues = Vec::new();

    let rendered = match kind {
        "text" => Rendered {
            body: Some(require_length(fields.get("body"), "text['body']", 4096)?),
            ..Rendered::of_kind("text")
        },
        "template" => render_template(fields, context.templates, &mut issues)?,
        "interactive" => render_interactive(fields)?,
        "reaction" => Rendered {
            body: Some(string(fields.get("emoji")).unwrap_or_default().to_owned()),
            ..Rendered::of_kind("reaction")
        },
        media if MEDIA_TYPES.contains(&media) => {
            let id = string(fields.get("id")).filter(|id| !id.is_empty());
            let link = string(fields.get("link")).filter(|link| !link.is_empty());

            if id.is_none() && link.is_none() {
                return Err(GraphError::new(
                    100,
                    format!("(#100) Param {media} must have either id or link"),
                ));
            }

            Rendered {
                caption: string(fields.get("caption")).map(str::to_owned),
                media: Some(MediaRef {
                    id: id.map(str::to_owned),
                    link: link.map(str::to_owned),
                    filename: string(fields.get("filename")).map(str::to_owned),
                    mime_type: None,
                }),
                ..Rendered::of_kind(media)
            }
        }
        other => Rendered {
            body: Some(Value::Object(fields.clone()).to_string()),
            ..Rendered::of_kind(other)
        },
    };

    let free_form = kind != "template" && kind != "reaction";
    let outside_window = free_form
        && context
            .last_inbound_at
            .is_none_or(|at| context.now - at > SERVICE_WINDOW_MS);

    if outside_window {
        issues.push(Issue::OutsideServiceWindow);
    }

    Ok(Outbound {
        kind: kind.to_owned(),
        contact,
        input: input.to_owned(),
        rendered,
        issues,
        context_id: object(payload.get("context"))
            .and_then(|context| string(context.get("message_id")))
            .map(str::to_owned),
        outside_window,
    })
}
