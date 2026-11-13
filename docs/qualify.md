# Static qualification

`engine::qualify::qualify` decides from what GitHub said about a ticket
whether kai may use it. It is a pure function: no repository, no VM, no
model, no network. The types it reads and returns are in the crate
`types`. The store keeps an `Issue` as GitHub said it, body and all;
`engine::ticket::from_issue` reads its body as the kai ticket form and
makes the `Ticket` that `qualify` judges.

## The types

```mermaid
%% checked against crates/types, crates/engine
classDiagram
    direction LR

    class Issue {
        +IssueId id
        +String title
        +String url
        +IssueState state
        +Option~String~ author
        +String created_at
        +String updated_at
        +String body
        +Vec~String~ labels
        +bool labels_complete
        +Vec~LabelEvent~ label_events
        +bool label_events_complete
    }
    class Ticket {
        +IssueId issue
        +String title
        +String url
        +IssueState state
        +Vec~String~ labels
        +bool labels_complete
        +Vec~LabelEvent~ label_events
        +bool label_events_complete
        +String updated_at
        +Option~Form~ form
    }
    class from_issue {
        <<function>>
        +from_issue(issue: &Issue) Ticket
    }
    class IssueId {
        +RepoName repo
        +u64 number
    }
    class Form {
        +Option~String~ instruction
        +Option~String~ repo
        +Option~String~ base_ref
        +Option~String~ check
        +Option~String~ hidden_tests
    }
    class LabelEvent {
        +String label
        +Option~String~ actor
        +String at
    }
    class IssueState {
        <<enumeration>>
        Open
        Closed
    }

    class qualify {
        <<function>>
        +qualify(ticket: &Ticket) Qualification
    }

    class Qualification {
        +Vec~Finding~ findings
        +verdict() Verdict
    }
    class Finding {
        +Rule rule
        +Outcome outcome
    }
    class Rule {
        <<enumeration>>
        Label
        TrustGate
        Open
        Form
        Instruction
        NoCodeBlock
        Repo
        Ref
        Check
        HiddenTests
        AtRef
    }
    class Outcome {
        <<enumeration>>
        Pass
        Fail(String)
        NotChecked(String)
    }
    class Verdict {
        <<enumeration>>
        Qualified
        Refused
        Undecided
    }

    Issue "1" *-- "1" IssueId
    Issue "1" *-- "*" LabelEvent
    Issue --> IssueState
    from_issue ..> Issue : reads
    from_issue ..> Ticket : returns
    Ticket "1" *-- "1" IssueId
    Ticket "1" *-- "0..1" Form : None when the body is not the form
    Ticket "1" *-- "*" LabelEvent
    Ticket --> IssueState
    qualify ..> Ticket : reads
    qualify ..> Qualification : returns
    Qualification "1" *-- "*" Finding
    Finding --> Rule
    Finding --> Outcome
    Qualification ..> Verdict : verdict()
    note for Outcome "Fail and NotChecked say why, in words"
```

A form field left empty is `None`; qualifying is what judges it. A
`Finding` is one rule applied to one ticket. A sync fetches at most 100
labels and 100 labeled events of an issue; `labels_complete` and
`label_events_complete` say whether that was all of them, and when the
events are cut off, the latest are the ones missing.

## What `qualify` does

```mermaid
%% drawn from engine::qualify::qualify#da99814f, engine::qualify::form_rules#90473c16
flowchart TD
    T([Ticket]) --> I["Issue rules, always applied:<br/>label, trust-gate, open"]
    I --> F{"Is the body the<br/>kai ticket form?"}
    F -- no --> FN["form: Fail<br/>the field rules are not applied"]
    F -- yes --> FY["form: Pass"]
    FY --> R["Field rules, one finding each:<br/>instruction, no-code-block, repo, ref,<br/>check, hidden-tests, at-ref"]
    FN --> Q([Qualification: the findings in this order])
    R --> Q
```

| Rule | Pass when | Fail when | Not checked when |
| --- | --- | --- | --- |
| `label` | the issue has the label `kai` | it has not, and every label was fetched | it has not among the 100 fetched, and it has more |
| `trust-gate` | | | always, for now: who put the label on is not looked up |
| `open` | the issue is open | it is closed | |
| `form` | the body has the form's `### Instruction` heading | it has not | |
| `instruction` | the instruction is not blank | it is empty or blank | |
| `no-code-block` | no line of the instruction starts with ```` ``` ```` or `~~~` | one does: it may give the solution away | |
| `repo` | the repository is `owner/name` | it is missing or malformed | |
| `ref` | the ref is a full commit sha (40 hex digits) | it is missing, a branch, or short | |
| `check` | the ticket names a check | | it names none: the ecosystem's default is not detected yet |
| `hidden-tests` | the ticket names hidden tests | it names none | |
| `at-ref` | | | always, for now: the repository is not looked at |

## The verdict

```mermaid
%% drawn from types::Qualification::verdict#a956ad69
flowchart TD
    Q([Qualification]) --> A{"Did any rule fail?"}
    A -- yes --> R([Refused])
    A -- no --> B{"Was any rule not checked?"}
    B -- yes --> U([Undecided])
    B -- no --> OK([Qualified])
```

A rule that was not checked is never counted as met. Since `trust-gate`
and `at-ref` are not checked for any ticket yet, every ticket is refused
or undecided; none is qualified until both are built.

## How kai uses it

`kai github sync` fills the store with issues as GitHub said them; with
`--full` it also removes those GitHub no longer returns. `kai ticket
qualify` reads the issues, makes each a ticket and judges it. Qualifying
needs no network: it works on what the last sync fetched, and `kai store
status` says when that was.

```mermaid
%% checked against crates/store, crates/github, crates/engine
%% drawn from cli::cmd::github::sync#96be4668, cli::cmd::ticket::print_qualify#bfc62412
sequenceDiagram
    actor You
    participant cli as kai (cli)
    participant gh as github::Client
    participant GitHub
    participant store as store::Store
    participant ticket as engine::ticket
    participant engine as engine::qualify

    You->>cli: kai github sync
    cli->>store: cursor(board)
    cli->>gh: fetch_board(board, since)
    gh->>GitHub: GraphQL, 50 issues a page
    GitHub-->>gh: issues, labels, labeled events
    gh-->>cli: issues and the next cursor
    cli->>store: record_sync(board, issues, cursor, full)

    You->>cli: kai ticket qualify
    cli->>store: issues()
    store-->>cli: every issue
    loop each issue
        cli->>ticket: from_issue(issue)
        ticket-->>cli: Ticket, the body read as the form
        cli->>engine: qualify(ticket)
        engine-->>cli: Qualification
        Note over cli: prints the verdict and each finding that did not pass
    end
    cli-->>You: how many are qualified, refused, undecided
```
