// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! `mule-assistant`: REPL de terminal o servidor WebSocket para controlar
//! eMuleRust mediante un LLM local (Ollama en loopback por defecto).

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use mule_assistant::config::{AssistantConfig, DEFAULT_ENDPOINT, DEFAULT_MODEL, DEFAULT_WS_PORT};
use mule_assistant::llm::ollama::OllamaBackend;
use mule_assistant::llm::openai::OpenAiCompatibleBackend;
use mule_assistant::llm::LlmBackend;
use mule_assistant::repl::run_repl;
use mule_assistant::session::Session;
use mule_assistant::ws::serve_ws;
use mule_daemon::{DaemonSession, WorkDirectoryResolver};
use mule_mcp::tools::ToolProfile;

#[derive(Parser, Debug)]
#[command(name = "mule-assistant")]
#[command(about = "Assistant IA local pour eMuleRust (tool-calling LLM → daemon RPC)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// URL RPC WebSocket explícita del daemon eMuleRust (ej. ws://127.0.0.1:4660/rpc)
    #[arg(long)]
    url: Option<String>,

    /// Token de autenticación de sesión para el daemon
    #[arg(long)]
    token: Option<String>,

    /// Ruta al directorio de trabajo de eMuleRust donde residen session.json y auth.token
    #[arg(long)]
    workdir: Option<PathBuf>,

    /// Ruta al ejecutable eMuleRust.exe o a la raíz del paquete de la aplicación
    #[arg(long)]
    app: Option<PathBuf>,

    /// URL base del backend LLM: requiere IP literal en bucle local a menos que
    /// se indique --allow-remote-endpoint (por defecto: http://127.0.0.1:11434)
    #[arg(long, default_value = DEFAULT_ENDPOINT)]
    endpoint: String,

    /// Nombre del modelo en el backend (debe admitir la llamada a herramientas de Ollama)
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    /// Tipo de proveedor LLM: 'ollama' (por defecto) u 'openai' (para LM Studio, LocalAI, vLLM, OpenAI)
    #[arg(long, default_value = "ollama")]
    provider: String,

    /// Clave API opcional para proveedores compatibles con OpenAI
    #[arg(long)]
    api_key: Option<String>,

    /// Permite explícitamente un punto de conexión LLM no local
    #[arg(long)]
    allow_remote_endpoint: bool,

    /// Techo de riesgo de herramientas presentado al modelo: read, safe, full
    #[arg(long, value_enum, default_value_t = ToolProfile::Full)]
    tool_profile: ToolProfile,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Sesión de terminal interactiva (por defecto)
    Chat,
    /// Petición puntual y posterior salida
    Prompt {
        /// Texto de la petición del usuario
        text: String,
    },
    /// Servidor WebSocket para la interfaz Flutter (ws://127.0.0.1:<port>/assistant)
    Serve {
        /// Puerto de bucle local al que vincularse
        #[arg(long, default_value_t = DEFAULT_WS_PORT)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Registros legibles en stderr exclusivamente; stdout sigue siendo la interfaz REPL.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mule_assistant=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let config = AssistantConfig {
        endpoint: cli.endpoint.clone(),
        model: cli.model.clone(),
        allow_remote_endpoint: cli.allow_remote_endpoint,
        tool_profile: cli.tool_profile,
        ..Default::default()
    };
    config.validate()?;

    let backend: Box<dyn LlmBackend> = match cli.provider.to_lowercase().as_str() {
        "openai" | "lmstudio" | "localai" => {
            Box::new(OpenAiCompatibleBackend::new(&config, cli.api_key.clone())?)
        }
        _ => Box::new(OllamaBackend::new(&config)?),
    };
    let session = Session::connect(
        backend,
        config.clone(),
        cli.url.clone(),
        cli.token.clone(),
        cli.workdir.clone(),
        cli.app.clone(),
    )
    .await?;

    info!(
        "session prête — {} outils visibles (profil {})",
        session.visible_tool_count(),
        cli.tool_profile.as_str()
    );

    match cli.command.clone().unwrap_or(Command::Chat) {
        Command::Chat => {
            let mut session = session;
            run_repl(&mut session).await?;
        }
        Command::Prompt { text } => {
            let mut session = session;
            let (events_tx, mut events_rx) = mpsc::unbounded_channel();
            let (confirm_tx, _confirm_rx) = mpsc::channel(8);
            // Sin respondedor de confirmación: las llamadas a confirmar se
            // rechazan (comportamiento seguro por defecto en modo no interactivo).
            let turn = session.user_turn(&text, &events_tx, &confirm_tx);
            tokio::pin!(turn);
            loop {
                tokio::select! {
                    res = &mut turn => {
                        while let Ok(ev) = events_rx.try_recv() {
                            print_event_oneshot(&ev);
                        }
                        res?;
                        break;
                    }
                    ev = events_rx.recv() => {
                        if let Some(e) = ev { print_event_oneshot(&e); }
                    }
                }
            }
        }
        Command::Serve { port } => {
            let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            let token = resolve_expected_token(&cli).ok_or(
                "jeton de session daemon introuvable — passer --token explicitement",
            )?;
            let listener = tokio::net::TcpListener::bind(bind).await?;
            serve_ws(listener, session, token).await?;
        }
    }
    Ok(())
}

/// Token esperado por `assistant.hello`: `--token` explícito o token de
/// la sesión del daemon en el directorio de trabajo; misma fuente que `DaemonClient`.
fn resolve_expected_token(cli: &Cli) -> Option<String> {
    if let Some(t) = &cli.token {
        return Some(t.clone());
    }
    let workdir = cli.workdir.clone().unwrap_or_else(|| {
        WorkDirectoryResolver::resolve(None, None).workdir
    });
    DaemonSession::read_token(&workdir)
}

/// Visualización compacta de un evento en modo puntual (one-shot).
fn print_event_oneshot(ev: &mule_assistant::AssistantEvent) {
    use mule_assistant::AssistantEvent as E;
    match ev {
        E::Token { content } => print!("{content}"),
        E::ToolStart { name, args } => println!("\n[outil →] {name} {args}"),
        E::ToolResult {
            name,
            is_error,
            content,
        } => println!("[résultat ←] {name} (err={is_error}): {content}"),
        E::Final { content } => println!("\n{content}"),
        E::Notice { message } | E::Error { message } => println!("[info] {message}"),
    }
    let _ = std::io::stdout().flush();
}
