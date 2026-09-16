use serde_json::{Value, json};

use super::anthropic::{Message, MessagesResponse};
use super::conversation::{ModelTransport, converse, thread};
use super::prompt::system_prompt;
use super::tools::{
    ANSWER_TOOL, CREATE_TOOL, LOOK_UP_TOOL, NAME_PATTERN, metric_query_schema, proposal_tools,
};
use super::*;

impl Turn {
    fn user(content: &str) -> Self {
        Self {
            role: "user".to_owned(),
            content: content.to_owned(),
        }
    }

    fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: content.to_owned(),
        }
    }
}

fn people() -> People {
    People::new("identity")
}

#[test]
fn an_answer_intent_carries_a_query_and_stores_nothing() {
    let reply = r#"{"intent":"answer","reply":"About 59 lines on the first day","query":{"table":"events","fields":[{"json":"day","type":"string","as_name":"day"},{"json":"lines","type":"int","agg":"sum","as_name":"lines"}],"group_by":["day"],"filters":[]}}"#;

    match Proposal::parse(reply, &people()).unwrap_or_else(|error| panic!("parses: {error}")) {
        Proposal::Answer { reply, query } => {
            assert_eq!(reply, "About 59 lines on the first day");
            let Some(query) = query else {
                panic!("this answer carries a query");
            };
            query
                .compile(&people())
                .unwrap_or_else(|error| panic!("the query compiles: {error}"));
        }
        Proposal::Create { .. } => panic!("expected an answer"),
    }
}

#[test]
fn a_create_intent_is_read_out_of_the_model_reply() {
    let reply = r#"{"intent":"create","reply":"Here you go","metric":{"name":"commits_per_day","body":{"table":"events","fields":[{"json":"day","type":"string","as_name":"day"}],"group_by":["day"],"filters":[]}},"widgets":[{"name":"commits_table","body":{"type":"table","metric":"commits_per_day","columns":["day"]}}],"dashboard":{"name":"engineering","body":{"title":"Engineering","widgets":["commits_table"]}}}"#;

    match Proposal::parse(reply, &people()).unwrap_or_else(|error| panic!("parses: {error}")) {
        Proposal::Create { reply, widgets, .. } => {
            assert_eq!(reply, "Here you go");
            assert_eq!(widgets.len(), 1);
        }
        Proposal::Answer { .. } => panic!("expected a creation"),
    }
}

#[test]
fn a_metric_the_compiler_refuses_is_not_stored() {
    let reply = r#"{"intent":"create","reply":"x","metric":{"name":"bad","body":{"table":"events`--","fields":[],"group_by":[],"filters":[]}},"widgets":[],"dashboard":null}"#;

    assert!(matches!(
        Proposal::parse(reply, &people()),
        Err(ChatError::Metric(_))
    ));
}

#[test]
fn a_question_about_the_data_itself_answers_without_a_query() {
    // The tool used to require a query, so a question the data cannot
    // answer got an invented one - and the reply carried a table of 31
    // rows of `count: 0` beneath it.
    let reply = json!({
        "intent": "answer",
        "reply": "You have events, with author, day, event and lines."
    })
    .to_string();

    let proposal = Proposal::parse(&reply, &people())
        .unwrap_or_else(|error| panic!("an answer needs no query: {error}"));

    assert!(matches!(proposal, Proposal::Answer { query: None, .. }));
}

/// The tool of that name, so a test does not break when the list grows.
fn tool(name: &str) -> Value {
    proposal_tools()
        .into_iter()
        .find(|tool| tool["name"] == json!(name))
        .unwrap_or_else(|| panic!("{name} is offered"))
}

#[test]
fn the_query_schema_offers_an_ordering() {
    let answer = tool(ANSWER_TOOL);
    let order = &answer["input_schema"]["properties"]["query"]["properties"]["order_by"];

    assert_eq!(order["properties"]["field"]["type"], json!("string"));
    assert_eq!(
        order["properties"]["direction"]["enum"],
        json!(["asc", "desc"])
    );
}

#[test]
fn the_answer_tool_asks_only_for_the_reply() {
    let answer = tool(ANSWER_TOOL);

    assert_eq!(answer["input_schema"]["required"], json!(["reply"]));
    // Still described, so the model knows a query is how it reads data.
    assert!(answer["input_schema"]["properties"]["query"].is_object());
}

#[test]
fn a_reply_the_model_encoded_twice_is_read_back_as_prose() {
    // Seen live: the whole reply arrived as a quoted JSON string, so the
    // panel showed literal \n between paragraphs and a trailing quote.
    let reply = json!({
        "intent": "answer",
        "reply": r#""One.\n\nTwo.""#
    })
    .to_string();

    let proposal = Proposal::parse(&reply, &people())
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

    let Proposal::Answer { reply, .. } = proposal else {
        panic!("expected an answer");
    };
    assert_eq!(reply, "One.\n\nTwo.");
}

#[test]
fn prose_that_merely_contains_quotes_is_left_alone() {
    let reply = json!({
        "intent": "answer",
        "reply": "The column is called \"day\"."
    })
    .to_string();

    let proposal = Proposal::parse(&reply, &people())
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

    let Proposal::Answer { reply, .. } = proposal else {
        panic!("expected an answer");
    };
    assert_eq!(reply, "The column is called \"day\".");
}

#[test]
fn the_whole_thread_reaches_the_model_with_the_new_turn_last() {
    let turns = [
        Turn::user("how many lines per day?"),
        Turn::assistant("Here they are."),
    ];

    let messages = thread(&turns, "and by author?");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "how many lines per day?");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "and by author?");
}

#[test]
fn a_first_message_is_a_thread_of_one() {
    let messages = thread(&[], "hello");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn the_prompt_names_what_is_already_built() {
    let prompt = system_prompt(
        &[KnownTable {
            name: "events".to_owned(),
            fields: "day (string)".to_owned(),
        }],
        &Catalogue {
            metrics: vec!["lines_per_day".to_owned()],
            widgets: vec!["lines_chart".to_owned()],
            dashboards: vec!["engineering".to_owned()],
        },
        "",
    );

    assert!(prompt.contains("lines_per_day"), "{prompt}");
    assert!(prompt.contains("lines_chart"), "{prompt}");
    assert!(prompt.contains("engineering"), "{prompt}");
}

#[test]
fn the_grammar_lets_a_metric_declare_the_clock_a_reader_windows_by() {
    let schema = metric_query_schema();
    let properties = &schema["properties"];

    assert!(properties.get("time").is_some(), "{schema}");
    assert!(properties.get("max_range").is_some(), "{schema}");
}

#[test]
fn the_prompt_asks_for_a_clock_when_the_table_carries_one() {
    let prompt = system_prompt(
        &[KnownTable {
            name: "events".to_owned(),
            fields: "occurred_at (datetime)".to_owned(),
        }],
        &Catalogue::default(),
        "",
    );

    assert!(prompt.contains("\"time\""), "{prompt}");
}

#[test]
fn an_empty_catalogue_says_nothing_is_built_yet() {
    let prompt = system_prompt(
        &[KnownTable {
            name: "events".to_owned(),
            fields: "day (string)".to_owned(),
        }],
        &Catalogue::default(),
        "",
    );

    assert!(prompt.contains("Nothing is built yet"), "{prompt}");
}

/// A model whose answers are decided in advance, so the conversation is
/// exercised without an HTTP server standing in for the API.
struct ScriptedModel {
    answers: std::sync::Mutex<std::collections::VecDeque<Value>>,
    seen: std::sync::Mutex<Vec<Vec<Message>>>,
}

impl ScriptedModel {
    fn new(answers: Vec<Value>) -> Self {
        Self {
            answers: std::sync::Mutex::new(answers.into_iter().collect()),
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn turns(&self) -> Vec<Vec<Message>> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl ModelTransport for ScriptedModel {
    async fn send(
        &self,
        _system: &str,
        messages: &[Message],
    ) -> Result<MessagesResponse, ChatError> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(messages.to_vec());
        let next = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        let Some(next) = next else {
            panic!("the model was called more times than the test scripted");
        };

        Ok(serde_json::from_value(next)
            .unwrap_or_else(|error| panic!("the scripted answer parses: {error}")))
    }
}

#[derive(Debug)]
struct FixedSchemas(&'static str);

#[async_trait::async_trait]
impl Schemas for FixedSchemas {
    async fn describe(&self, _tables: &[String]) -> String {
        self.0.to_owned()
    }
}

fn look_up_turn(id: &str, tables: &Value) -> Value {
    json!({ "content": [
        { "type": "tool_use", "id": id, "name": LOOK_UP_TOOL, "input": { "tables": tables } },
    ]})
}

fn answer_turn(reply: &str) -> Value {
    json!({ "content": [
        { "type": "tool_use", "id": "t2", "name": ANSWER_TOOL, "input": { "reply": reply } },
    ]})
}

#[tokio::test]
async fn a_schema_lookup_is_answered_and_the_turn_continues() {
    let model = ScriptedModel::new(vec![
        look_up_turn("t1", &json!(["silver.git_commits"])),
        answer_turn("There are 12 commits."),
    ]);

    let proposal = converse(
        &model,
        &FixedSchemas("silver.git_commits\n  author_email String\n"),
        "system",
        vec![Message::user("how many commits?")],
        &[],
        &people(),
    )
    .await
    .unwrap_or_else(|error| panic!("the turn completes: {error}"));

    let Proposal::Answer { reply, .. } = proposal else {
        panic!("expected an answer");
    };
    assert_eq!(reply, "There are 12 commits.");

    // The second call carries the question, the model's own tool_use, and
    // the result addressed to it — the API refuses any other order.
    let second = &model.turns()[1];
    assert_eq!(second.len(), 3);
    assert_eq!(second[1].role, "assistant");
    assert_eq!(second[1].content[0]["type"], "tool_use");
    assert_eq!(second[1].content[0]["id"], "t1");
    assert_eq!(second[2].role, "user");
    assert_eq!(second[2].content[0]["type"], "tool_result");
    assert_eq!(second[2].content[0]["tool_use_id"], "t1");
    assert!(
        second[2].content[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("author_email"),
        "the columns must reach the model"
    );
}

#[tokio::test]
async fn a_rejected_proposal_is_refused_to_the_call_that_made_it() {
    // The API answers 400 when a tool_use is followed by anything but a
    // tool_result for it, which is how the repair round used to fail.
    let model = ScriptedModel::new(vec![
        json!({ "content": [
            { "type": "tool_use", "id": "bad", "name": ANSWER_TOOL, "input": {} },
        ]}),
        answer_turn("Corrected."),
    ]);

    let proposal = converse(
        &model,
        &FixedSchemas("unused"),
        "system",
        vec![Message::user("ask something")],
        &[],
        &people(),
    )
    .await
    .unwrap_or_else(|error| panic!("the repair round completes: {error}"));

    assert!(matches!(proposal, Proposal::Answer { .. }));

    let repair = &model.turns()[1];
    let result = &repair[2].content[0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "bad");
    assert_eq!(result["is_error"], true);
    assert!(
        result["content"]
            .as_str()
            .unwrap_or_default()
            .contains("rejected"),
        "the model must be told what was wrong"
    );
}

#[tokio::test]
async fn a_model_that_only_ever_looks_up_is_stopped() {
    // Four lookups is one past the budget, so the turn ends rather than
    // spending the reader's money in a circle.
    let model = ScriptedModel::new(vec![
        look_up_turn("t1", &json!(["silver.a"])),
        look_up_turn("t2", &json!(["silver.b"])),
        look_up_turn("t3", &json!(["silver.c"])),
        look_up_turn("t4", &json!(["silver.d"])),
    ]);

    let refused = converse(
        &model,
        &FixedSchemas("columns"),
        "system",
        vec![Message::user("go round in circles")],
        &[],
        &people(),
    )
    .await;

    assert!(matches!(refused, Err(ChatError::TooManyLookups)));
}

#[tokio::test]
async fn an_answer_needs_no_lookup_at_all() {
    let model = ScriptedModel::new(vec![answer_turn("Straight to it.")]);

    let proposal = converse(
        &model,
        &FixedSchemas("unused"),
        "system",
        vec![Message::user("what data is there?")],
        &[],
        &people(),
    )
    .await
    .unwrap_or_else(|error| panic!("the turn completes: {error}"));

    assert!(matches!(proposal, Proposal::Answer { .. }));
    assert_eq!(model.turns().len(), 1);
}

#[test]
fn a_lookup_asking_for_nothing_is_not_a_lookup() {
    // An empty list would spend a round trip and teach the model nothing.
    let empty: MessagesResponse = serde_json::from_value(look_up_turn("t1", &json!([])))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

    assert!(empty.look_up().is_none());
}

#[test]
fn the_map_and_both_field_shapes_are_explained() {
    let prompt = system_prompt(
        &[],
        &Catalogue::default(),
        "Gold (published metrics)\n  insight:\n    git_metric_observations\n",
    );

    assert!(prompt.contains("git_metric_observations"), "{prompt}");
    assert!(prompt.contains("look_up"), "{prompt}");
    // Which shape belongs to which table is the thing it gets wrong.
    assert!(prompt.contains("`column`"), "{prompt}");
    assert!(prompt.contains("`json`"), "{prompt}");
}

#[test]
fn the_widget_schema_offers_exactly_the_kinds_the_renderer_draws() {
    // The schema the model writes against and the switch that draws the
    // result are two views of one vocabulary. Every bug here came from
    // those two drifting apart.
    let created = tool(CREATE_TOOL);
    let widget = &created["input_schema"]["properties"]["widgets"]["items"]["properties"]["body"]["properties"];

    assert_eq!(
        widget["type"]["enum"],
        json!(["table", "line", "bar", "area", "stat", "pie"])
    );
    for field in ["metric", "columns", "x", "y", "value", "label"] {
        assert!(widget[field].is_object(), "{field} is offered");
    }
}

#[test]
fn the_query_schema_offers_both_a_database_and_a_column() {
    let answer = tool(ANSWER_TOOL);
    let query = &answer["input_schema"]["properties"]["query"]["properties"];

    assert_eq!(query["database"]["type"], json!("string"));
    let field = &query["fields"]["items"]["properties"];
    assert_eq!(field["person"]["enum"], json!(["email", "id"]));
    assert!(
        field["when"]["items"].is_object(),
        "a field can carry its own condition"
    );
    assert!(
        field["divide"]["items"].is_object(),
        "a field can divide two others"
    );
    assert_eq!(field["percent"]["type"], json!("boolean"));
    assert_eq!(field["column"]["type"], json!("string"));
    // Neither is required: exactly one of them is, which no JSON schema
    // this API accepts can express, so the compiler refuses it instead.
    assert_eq!(
        query["fields"]["items"]["required"],
        json!(["type", "as_name"])
    );
}

#[test]
fn a_lookup_tool_is_offered() {
    let look_up = tool(LOOK_UP_TOOL);

    assert_eq!(
        look_up["input_schema"]["properties"]["tables"]["type"],
        json!("array")
    );
    assert_eq!(look_up["input_schema"]["required"], json!(["tables"]));
}

#[test]
fn the_prompt_names_every_table_with_its_fields() {
    let prompt = system_prompt(
        &[KnownTable {
            name: "events".to_owned(),
            fields: "day (string), lines (int)".to_owned(),
        }],
        &Catalogue::default(),
        "",
    );

    assert!(
        prompt.contains("- events: day (string), lines (int)"),
        "{prompt}"
    );
    assert!(!prompt.contains("No tables are known yet"), "{prompt}");
}

#[test]
fn an_answer_naming_a_table_the_reader_does_not_have_is_refused() {
    let known = ["events".to_owned(), "silver.class_git_commits".to_owned()];
    let reply = json!({
        "intent": "answer",
        "reply": "here",
        "query": {
            "table": "information_schema_tables",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": []
        }
    })
    .to_string();

    let Err(rejection) = Proposal::checked(&reply, &known, &people()) else {
        panic!("a table that does not exist must not reach the database");
    };

    // The message is what the repair round hands back, so it has to name
    // the tables that DO exist.
    let feedback = rejection.feedback();
    assert!(feedback.contains("information_schema_tables"), "{feedback}");
    assert!(feedback.contains("events"), "{feedback}");
}

#[test]
fn a_known_table_passes_the_check() {
    let known = ["events".to_owned()];
    let reply = json!({
        "intent": "answer",
        "reply": "here",
        "query": {
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": []
        }
    })
    .to_string();

    assert!(matches!(
        Proposal::checked(&reply, &known, &people()),
        Ok(Proposal::Answer { .. })
    ));
}

#[test]
fn a_layer_table_passes_when_its_database_qualifies_it() {
    let known = ["silver.class_git_commits".to_owned()];
    let reply = json!({
        "intent": "answer",
        "reply": "here",
        "query": {
            "database": "silver",
            "table": "class_git_commits",
            "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
            "group_by": [],
            "filters": []
        }
    })
    .to_string();

    assert!(matches!(
        Proposal::checked(&reply, &known, &people()),
        Ok(Proposal::Answer { .. })
    ));
}

#[test]
fn the_same_table_in_another_database_is_refused() {
    // Two layers can hold a table of one name, so the database is part of
    // what is checked rather than dropped.
    let known = ["silver.class_git_commits".to_owned()];
    let reply = json!({
        "intent": "answer",
        "reply": "here",
        "query": {
            "database": "bronze_github",
            "table": "class_git_commits",
            "fields": [{ "column": "author_email", "type": "string", "as_name": "author" }],
            "group_by": [],
            "filters": []
        }
    })
    .to_string();

    assert!(matches!(
        Proposal::checked(&reply, &known, &people()),
        Err(ChatError::UnknownTable { .. })
    ));
}

#[test]
fn with_nothing_ingested_the_check_stands_aside() {
    // Refusing every table when we know of none would block the chat
    // outright on a stand whose listing failed.
    let reply = json!({
        "intent": "answer",
        "reply": "here",
        "query": {
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": [],
            "filters": []
        }
    })
    .to_string();

    assert!(matches!(
        Proposal::checked(&reply, &[], &people()),
        Ok(Proposal::Answer { .. })
    ));
}

#[test]
fn one_tool_per_intent_carries_the_name_charset() {
    let tools = proposal_tools();

    // One tool per intent, plus the lookup that ends no turn.
    assert_eq!(tools.len(), 3);
    assert!(tools.iter().any(|tool| tool["name"] == json!(ANSWER_TOOL)));
    assert!(tools.iter().any(|tool| tool["name"] == json!(CREATE_TOOL)));

    // Not strict on purpose: the nested MetricQuery exceeds the API's
    // compiled-grammar budget and a strict request is refused outright.
    // Verified by hand against the live API before this was written.
    for offered in &tools {
        assert!(offered.get("strict").is_none(), "strict must stay off");
        assert_eq!(
            offered["input_schema"]["additionalProperties"],
            json!(false)
        );
    }

    // Every name the model invents carries the charset DefinitionName
    // enforces, stated where the model reads it.
    let created = tool(CREATE_TOOL);
    let create = &created["input_schema"]["properties"];
    for path in [
        &create["metric"]["properties"]["name"],
        &create["dashboard"]["properties"]["name"],
        &create["widgets"]["items"]["properties"]["name"],
    ] {
        assert_eq!(path["pattern"], json!(NAME_PATTERN), "missing name pattern");
    }
}

#[test]
fn the_chosen_tool_becomes_the_intent() {
    let answered: MessagesResponse = serde_json::from_value(json!({
        "content": [
            { "type": "text", "text": "looking that up" },
            { "type": "tool_use", "name": ANSWER_TOOL,
              "input": { "reply": "here", "query": {} } },
        ],
    }))
    .unwrap_or_else(|error| panic!("the fixture deserializes: {error}"));

    let json = answered.proposal_json();
    assert!(json.contains("\"intent\":\"answer\""), "got {json}");
    assert!(!json.contains("looking that up"), "text leaked in");

    let created: MessagesResponse = serde_json::from_value(json!({
        "content": [
            { "type": "tool_use", "name": CREATE_TOOL,
              "input": { "reply": "made it", "widgets": [] } },
        ],
    }))
    .unwrap_or_else(|error| panic!("the fixture deserializes: {error}"));

    assert!(created.proposal_json().contains("\"intent\":\"create\""));
}

#[test]
fn prose_around_the_json_is_tolerated() {
    let reply = "Sure!\n```json\n{\"intent\":\"create\",\"reply\":\"ok\",\"widgets\":[],\"dashboard\":{\"name\":\"lines\",\"body\":{\"title\":\"Lines\",\"widgets\":[]}}}\n```";

    assert!(matches!(
        Proposal::parse(reply, &people()),
        Ok(Proposal::Create { .. })
    ));
}

#[test]
fn a_create_that_stores_nothing_is_refused() {
    let reply = json!({ "intent": "create", "reply": "done", "widgets": [] }).to_string();

    assert!(matches!(
        Proposal::parse(&reply, &people()),
        Err(ChatError::EmptyCreate)
    ));
}

#[test]
fn an_answer_with_a_field_outside_a_selected_group_by_is_a_metric_refusal() {
    let reply = json!({
        "intent": "answer",
        "reply": "x",
        "query": {
            "table": "events",
            "fields": [{ "json": "day", "type": "string", "as_name": "day" }],
            "group_by": ["not_selected"],
            "filters": []
        }
    })
    .to_string();

    assert!(matches!(
        Proposal::parse(&reply, &people()),
        Err(ChatError::Metric(MetricQueryError::GroupBy(_)))
    ));
}

#[tokio::test]
async fn a_blank_key_says_so_instead_of_answering() {
    let client = ChatClient::new(&SecretString::from("   ".to_owned()), "model".to_owned());

    let refusal = client
        .propose(&Ask {
            message: "commits per day",
            turns: &[],
            tables: &[],
            catalogue: &Catalogue::default(),
            map: "",
            allowed: &[],
            schemas: &FixedSchemas("unused"),
            people: &people(),
        })
        .await;

    assert!(matches!(refusal, Err(ChatError::NoKey)));
}
