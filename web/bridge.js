// Ponte entre a página (web/app.js, web/desktop.js) e o programa Tauri (src-tauri/src/main.rs).
// Substitui o preload.js do Electron: expõe o MESMO objeto window.astra, então a página
// não precisa mudar. Roda antes de qualquer script da página (initialization_script).
(function () {
  "use strict";
  const boot = window.__ASTRA_BOOT__ || {};
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args);
  const escutar = (evento, cb) => T.event.listen(evento, (e) => cb(e.payload));

  // Toque de verdade: só eventos vindos do sistema (isTrusted). Script da página não consegue fingir.
  const tocar = () => { invoke("tocou").catch(() => {}); };
  window.addEventListener("pointerdown", (e) => { if (e.isTrusted) tocar(); }, true);
  window.addEventListener("keydown", (e) => { if (e.isTrusted) tocar(); }, true);

  let aoSom = null, aoDesempenho = null, aoTeclas = null, aoProgresso = null;
  let aoMax = null, aoEmergencia = null;

  const base64ParaBytes = (b64) => {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  };

  escutar("som-dados", (b64) => { if (aoSom) aoSom(base64ParaBytes(b64)); });
  escutar("desempenho-dados", (d) => { if (aoDesempenho) aoDesempenho(d); });
  escutar("teclas-dados", (lista) => { if (aoTeclas) aoTeclas(lista); });
  escutar("atualizar-progresso", (p) => { if (aoProgresso) aoProgresso(p); });
  escutar("janela-max", (sim) => { if (aoMax) aoMax(!!sim); });
  escutar("emergencia", () => { if (aoEmergencia) aoEmergencia(); });
  // O menu do ícone pede para transmitir/parar: aciona o botão da página (como antes).
  escutar("menu-tela", () => { const b = document.getElementById("bt-tela"); if (b) b.click(); });

  window.astra = {
    servidor: boot.servidor,
    site: boot.site,
    versao: boot.versao,
    aparelho: boot.aparelho || "",
    usuario: "",
    atualizar: () => invoke("atualizar"),
    aoProgressoAtualizacao: (cb) => { aoProgresso = cb; },
    aviso: (titulo, texto) => { invoke("aviso", { titulo: String(titulo || ""), texto: String(texto || "") }); },
    transmitindo: (sim) => { invoke("transmitindo", { sim: !!sim }); },
    relaxado: (sim) => { invoke("relaxado", { sim: !!sim }); },
    acorde: (nome) => { invoke("acorde", { nome: String(nome || "") }); },
    mostrar: () => { invoke("mostrar_janela"); },
    janela: (acao) => { invoke("janela", { acao: String(acao) }); },
    protegerTela: (sim) => { invoke("proteger_tela", { sim: !!sim }); },
    gravadorNvidia: () => invoke("gravador_nvidia"),
    aoMaximizar: (f) => { aoMax = f; },
    miniJanela: false,   // TODO: a mini tela (window.open "astra-mini") ainda não existe no Tauri
    popupChat: (m) => {
      invoke("popup_chat", { m: {
        nome: String(m.nome || ""), texto: String(m.texto || ""), cor: String(m.cor || ""),
        foto: String(m.foto || ""), imagem: String(m.imagem || ""), gif: !!m.gif, tipo: String(m.tipo || ""),
      } });
    },
    listarFontes: () => invoke("listar_fontes"),
    definirFonte: (escolha) => invoke("definir_fonte", {
      escolha: { id: String((escolha && escolha.id) || ""), som: String((escolha && escolha.som) || "") },
    }),
    // desktop.js espera { ok, erro }; o comando Rust devolve Ok (vazio) ou Err (texto).
    iniciarSom: () => invoke("som_iniciar").then(
      () => ({ ok: true }),
      (erro) => ({ ok: false, erro: String(erro || "O capturador de som não abriu.") })),
    pararSom: () => { invoke("som_parar"); },
    aoReceberSom: (cb) => { aoSom = cb; },
    silencio: (sim) => { invoke("silencio", { sim: !!sim }); },
    risco: (r) => { invoke("risco", { r }); },
    teclaEmergencia: (acel) => invoke("tecla_emergencia", { acel: String(acel || "") }),
    aoEmergencia: (f) => { aoEmergencia = f; },
    teclas: (sim) => { invoke("teclas", { sim: !!sim }); },
    aoTeclas: (cb) => { aoTeclas = cb; },
    semMouse: { ligadoAgora: false, salvar: () => {} },
    reiniciar: () => { invoke("reiniciar"); },
    desempenho: (ligado) => { invoke("desempenho", { ligado: !!ligado }); },
    aoDesempenho: (cb) => { aoDesempenho = cb; },
    conferirModulo: (nome, texto, assinatura) => invoke("conferir_modulo", {
      nome: String(nome || ""), texto: String(texto || ""), assinatura: String(assinatura || ""),
    }),
    modulosAssinados: true,
  };
})();
