// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! REPL interactivo: transporte de terminal de la sesión.
//!
//! Muestra los tokens sobre la marcha, notifica las llamadas a herramientas
//! con sus argumentos exactos y gestiona las confirmaciones `o/N` sin
//! bloquear el bucle: stdin sigue siendo el único lector y las lecturas de
//! confirmación se realizan en la misma tarea que la entrada principal.

use std::io::Write;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::error::AssistantError;
use crate::session::{AssistantEvent, ConfirmRequest, Session};

/// Longitud máxima mostrada de un resultado de herramienta en el terminal:
/// la versión completa sigue disponible a través del WS/eventos.
const RESULT_DISPLAY_MAX_CHARS: usize = 1000;

/// Bucle REPL: `> prompt` → turno → visualización. `quit`/`exit`/EOF
/// finalizan limpiamente.
pub async fn run_repl(session: &mut Session) -> Result<(), AssistantError> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AssistantEvent>();
    let (confirm_tx, mut confirm_rx) = mpsc::channel::<ConfirmRequest>(8);

    println!("Assistant eMuleRust — {} outils visibles. 'quit' pour sortir.", session.visible_tool_count());

    loop {
        print!("> ");
        let _ = std::io::stdout().flush();
        line.clear();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|e| AssistantError::Transport(format!("lecture stdin : {e}")))?;
        if read == 0 {
            break; // EOF
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "quit" || input == "exit" {
            break;
        }

        if let Err(e) = run_one_turn(session, input, &events_tx, &confirm_tx, &mut events_rx, &mut confirm_rx, &mut reader).await {
            println!("[erreur] {e}");
        }
        println!();
    }
    Ok(())
}

/// Un turno: orquesta la visualización de los eventos y las lecturas de
/// confirmación mientras `user_turn` se ejecuta.
async fn run_one_turn(
    session: &mut Session,
    input: &str,
    events_tx: &mpsc::UnboundedSender<AssistantEvent>,
    confirm_tx: &mpsc::Sender<ConfirmRequest>,
    events_rx: &mut mpsc::UnboundedReceiver<AssistantEvent>,
    confirm_rx: &mut mpsc::Receiver<ConfirmRequest>,
    reader: &mut BufReader<tokio::io::Stdin>,
) -> Result<(), AssistantError> {
    let turn = session.user_turn(input, events_tx, confirm_tx);
    tokio::pin!(turn);

    loop {
        tokio::select! {
            res = &mut turn => {
                while let Ok(ev) = events_rx.try_recv() {
                    print_event(&ev);
                }
                res?;
                break;
            }
            ev = events_rx.recv() => {
                if let Some(e) = ev {
                    print_event(&e);
                }
            }
            req = confirm_rx.recv() => {
                if let Some(r) = req {
                    let allow = prompt_confirmation(&r, reader).await?;
                    let _ = r.respond.send(allow);
                }
            }
        }
    }
    Ok(())
}

/// Mensaje interactivo `o/N` con los argumentos exactos de la llamada.
async fn prompt_confirmation(
    req: &ConfirmRequest,
    reader: &mut BufReader<tokio::io::Stdin>,
) -> Result<bool, AssistantError> {
    println!(
        "\n[confirmation] {} — arguments : {}",
        req.call.name, req.call.args
    );
    print!("Exécuter cet appel ? [o/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    reader
        .read_line(&mut answer)
        .await
        .map_err(|e| AssistantError::Transport(format!("lecture stdin : {e}")))?;
    Ok(matches!(
        answer.trim().to_lowercase().as_str(),
        "o" | "oui" | "y" | "yes"
    ))
}

/// Representación en terminal de un evento.
fn print_event(ev: &AssistantEvent) {
    match ev {
        AssistantEvent::Token { content } => {
            print!("{content}");
            let _ = std::io::stdout().flush();
        }
        AssistantEvent::ToolStart { name, args } => {
            println!("\n[outil →] {name} {args}");
        }
        AssistantEvent::ToolResult {
            name,
            is_error,
            content,
        } => {
            let tag = if *is_error { "erreur" } else { "résultat" };
            let shown: String = content.chars().take(RESULT_DISPLAY_MAX_CHARS).collect();
            let suffix = if content.chars().count() > RESULT_DISPLAY_MAX_CHARS {
                "…"
            } else {
                ""
            };
            println!("[{tag} ←] {name}: {shown}{suffix}");
        }
        AssistantEvent::Final { content } => {
            println!("\n{content}");
        }
        AssistantEvent::Notice { message } => println!("[info] {message}"),
        AssistantEvent::Error { message } => println!("[erreur] {message}"),
    }
}
