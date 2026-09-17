# Element X iOS Integration Guide: Adding a Native Bot Tab

This guide documents how to build and integrate an additional native **SwiftUI tab inside [`element-x-ios`](https://github.com/element-hq/element-x-ios)** that connects directly to this `matrix-bot` backend.

Because `element-x-ios` is already authenticated with the user's Matrix account, the user **never sees a login screen, never enters a password, and never needs a Passkey**.

---

## 🏗️ Architecture Overview

```
┌────────────────────────────────────────────────────────────────────────┐
│                          Element X iOS                                 │
│                                                                        │
│  ┌────────────────────────┐         ┌───────────────────────────────┐  │
│  │   Existing Chats Tab   │         │    New "Assistant" Tab (UI)   │  │
│  └───────────┬────────────┘         └───────────────┬───────────────┘  │
│              │                                      │                  │
│              ▼                                      ▼                  │
│  ┌────────────────────────┐         ┌───────────────────────────────┐  │
│  │    MatrixRustSDK       │         │       BotClient (Actor)       │  │
│  │ (Active Matrix Session)│         │ (Handles API & Auto-Auth)     │  │
│  └───────────┬────────────┘         └───────────────┬───────────────┘  │
└──────────────┼──────────────────────────────────────┼──────────────────┘
               │ Matrix Sync (APNs)                   │ REST / SSE
               ▼                                      ▼
┌───────────────────────────┐         ┌──────────────────────────────────┐
│     Matrix Homeserver     │         │        matrix-bot Backend        │
│    (Synapse / Conduit)    │◄────────┤         (Axum REST API)          │
│                           │ whoami  │                                  │
│  • Manages push tokens    │ verify  │  • Issues Bearer session_token   │
│  • Sends APNs to iOS      │         │  • Persists Todos, Reminders, AI │
└───────────────────────────┘         └──────────────────────────────────┘
```

---

## 🔑 1. Authentication Handshake (`POST /api/v1/auth/matrix/login`)

When your tab appears in Element X, it performs a transparent 1-step handshake:

1. Element X reads the active Matrix credentials already in memory:
   - Access token: `clientProxy.client.accessToken()`
   - Homeserver URL: `clientProxy.homeserverURL.absoluteString`
2. Element X calls `POST /api/v1/auth/matrix/login`:
   ```json
   {
     "access_token": "syt_...",
     "homeserver": "https://mbm.one"
   }
   ```
3. The backend calls the homeserver's `/_matrix/client/v3/account/whoami` to verify that the token belongs to `@user:domain`.
4. The backend resolves or provisions the account in SQLite via `AccountRepository::resolve_or_provision`.
5. The backend returns a local `session_token`:
   ```json
   {
     "session_token": "mb_sess_...",
     "account_id": "acct_...",
     "display_name": "Evan",
     "mxid": "@evan:mbm.one"
   }
   ```
6. All subsequent requests pass `Authorization: Bearer <session_token>`.

---

## 🛠️ 2. Swift Networking Client (`BotClient.swift`)

Create this file in your Element X project (e.g. `ElementX/Sources/Services/Bot/BotClient.swift`):

```swift
import Foundation

public actor BotClient {
    public static let shared = BotClient()

    // Base URL of your running matrix-bot backend
    private let baseURL: URL
    private var sessionToken: String?
    private let jsonDecoder: JSONDecoder
    private let jsonEncoder: JSONEncoder

    public init(baseURL: URL = URL(string: "https://bot.yourdomain.com/api/v1")!) {
        self.baseURL = baseURL
        self.jsonDecoder = JSONDecoder()
        self.jsonDecoder.dateDecodingStrategy = .iso8601
        self.jsonEncoder = JSONEncoder()
        self.jsonEncoder.dateEncodingStrategy = .iso8601
    }

    // MARK: - Models

    public struct MatrixLoginRequest: Encodable {
        public let access_token: String
        public let homeserver: String?
    }

    public struct MatrixLoginResponse: Decodable {
        public let session_token: String
        public let account_id: String
        public let display_name: String
        public let mxid: String
    }

    public struct TodoItem: Identifiable, Codable {
        public let id: Int64
        public let text: String
        public let priority: String
        public let completed: Bool
        public let due_date: String?
    }

    public struct CreateTodoRequest: Encodable {
        public let text: String
        public let priority: String
    }

    public struct CreateReminderRequest: Encodable {
        public let message: String
        public let seconds: Int
        public let room_id: String?
    }

    public struct ChatRequest: Encodable {
        public let message: String
        public let conversation_id: String?
    }

    public struct ChatResponse: Decodable {
        public let response: String
        public let conversation_id: String
    }

    // MARK: - Authentication

    /// Exchanges active Element X Matrix credentials for a bot session token
    public func authenticate(accessToken: String, homeserver: String) async throws -> MatrixLoginResponse {
        let payload = MatrixLoginRequest(access_token: accessToken, homeserver: homeserver)
        let body = try jsonEncoder.encode(payload)

        var request = URLRequest(url: baseURL.appendingPathComponent("auth/matrix/login"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = body

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpRes = response as? HTTPURLResponse, httpRes.statusCode == 200 else {
            throw URLError(.userAuthenticationRequired)
        }

        let loginResponse = try jsonDecoder.decode(MatrixLoginResponse.self, from: data)
        self.sessionToken = loginResponse.session_token
        return loginResponse
    }

    // MARK: - Authenticated Request Helper

    private func performRequest(endpoint: String, method: String = "GET", body: Data? = nil) async throws -> Data {
        guard let sessionToken else {
            throw URLError(.userAuthenticationRequired)
        }

        var req = URLRequest(url: baseURL.appendingPathComponent(endpoint))
        req.httpMethod = method
        req.setValue("Bearer \(sessionToken)", forHTTPHeaderField: "Authorization")
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = body
        }

        let (data, response) = try await URLSession.shared.data(for: req)
        guard let httpRes = response as? HTTPURLResponse else {
            throw URLError(.badServerResponse)
        }

        guard (200...299).contains(httpRes.statusCode) else {
            if httpRes.statusCode == 401 {
                self.sessionToken = nil
                throw URLError(.userAuthenticationRequired)
            }
            throw URLError(.badServerResponse)
        }

        return data
    }

    // MARK: - API Methods

    public func getTodos() async throws -> [TodoItem] {
        let data = try await performRequest(endpoint: "todos")
        return try jsonDecoder.decode([TodoItem].self, from: data)
    }

    public func createTodo(text: String, priority: String = "medium") async throws -> TodoItem {
        let body = try jsonEncoder.encode(CreateTodoRequest(text: text, priority: priority))
        let data = try await performRequest(endpoint: "todos", method: "POST", body: body)
        return try jsonDecoder.decode(TodoItem.self, from: data)
    }

    public func createReminder(message: String, seconds: Int, roomID: String? = nil) async throws {
        let body = try jsonEncoder.encode(CreateReminderRequest(message: message, seconds: seconds, room_id: roomID))
        _ = try await performRequest(endpoint: "reminders", method: "POST", body: body)
    }

    public func askAI(message: String, conversationID: String? = nil) async throws -> ChatResponse {
        let body = try jsonEncoder.encode(ChatRequest(message: message, conversation_id: conversationID))
        let data = try await performRequest(endpoint: "ai/chat", method: "POST", body: body)
        return try jsonDecoder.decode(ChatResponse.self, from: data)
    }
}
```

---

## 📱 3. SwiftUI Tab View (`AssistantTabView.swift`)

Create this view in your Element X project:

```swift
import SwiftUI

public struct AssistantTabView: View {
    // Injected from Element X's coordinator
    let accessToken: String
    let homeserverURL: String
    let botRoomID: String? // Optional: 1:1 direct chat room ID with the bot

    @State private var isAuthenticated = false
    @State private var todos: [BotClient.TodoItem] = []
    @State private var newTodoText = ""
    @State private var aiInput = ""
    @State private var aiResponses: [String] = []
    @State private var isLoading = false

    public init(accessToken: String, homeserverURL: String, botRoomID: String? = nil) {
        self.accessToken = accessToken
        self.homeserverURL = homeserverURL
        self.botRoomID = botRoomID
    }

    public var body: some View {
        NavigationStack {
            List {
                // Section: AI Assistant Quick Chat
                Section("AI Assistant") {
                    ForEach(aiResponses, id: \.self) { reply in
                        Text(reply)
                            .font(.subheadline)
                            .foregroundStyle(.primary)
                    }

                    HStack {
                        TextField("Ask assistant...", text: $aiInput)
                            .textFieldStyle(.roundedBorder)
                        Button("Send") {
                            sendAiMessage()
                        }
                        .disabled(aiInput.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }

                // Section: Todos
                Section("Tasks") {
                    HStack {
                        TextField("Add new task...", text: $newTodoText)
                        Button(action: addTodo) {
                            Image(systemName: "plus.circle.fill")
                        }
                        .disabled(newTodoText.trimmingCharacters(in: .whitespaces).isEmpty)
                    }

                    if todos.isEmpty {
                        Text("No tasks yet.")
                            .foregroundStyle(.secondary)
                            .font(.footnote)
                    } else {
                        ForEach(todos) { item in
                            HStack {
                                Image(systemName: item.completed ? "checkmark.circle.fill" : "circle")
                                Text(item.text)
                                    .strikethrough(item.completed)
                                Spacer()
                                Text(item.priority.uppercased())
                                    .font(.caption2.monospaced())
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }

                // Section: Quick Reminders
                Section("Quick Reminder") {
                    Button("Remind me in 30 minutes") {
                        Task {
                            try? await BotClient.shared.createReminder(
                                message: "Follow up on Element task",
                                seconds: 1800,
                                roomID: botRoomID
                            )
                        }
                    }
                }
            }
            .navigationTitle("Assistant")
            .task {
                await loginAndLoadData()
            }
            .refreshable {
                await loadTodos()
            }
        }
    }

    private func loginAndLoadData() async {
        guard !isAuthenticated else { return }
        isLoading = true
        do {
            _ = try await BotClient.shared.authenticate(
                accessToken: accessToken,
                homeserver: homeserverURL
            )
            isAuthenticated = true
            await loadTodos()
        } catch {
            print("Failed to authenticate with bot backend: \(error)")
        }
        isLoading = false
    }

    private func loadTodos() async {
        do {
            todos = try await BotClient.shared.getTodos()
        } catch {
            print("Failed to load todos: \(error)")
        }
    }

    private func addTodo() {
        let text = newTodoText.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return }
        newTodoText = ""

        Task {
            if let newTodo = try? await BotClient.shared.createTodo(text: text) {
                todos.insert(newTodo, at: 0)
            }
        }
    }

    private func sendAiMessage() {
        let msg = aiInput.trimmingCharacters(in: .whitespaces)
        guard !msg.isEmpty else { return }
        aiInput = ""
        aiResponses.append("You: \(msg)")

        Task {
            if let res = try? await BotClient.shared.askAI(message: msg) {
                aiResponses.append("Bot: \(res.response)")
            }
        }
    }
}
```

---

## 🧭 4. Registering the Tab in `element-x-ios` Navigation

In `element-x-ios`, the main navigation lives in `HomeScreenCoordinator.swift` (or the top-level tab container `MainCoordinator.swift`):

1. **Locate the Main Navigation Flow**:
   Open `ElementX/Sources/Screens/HomeScreen/HomeScreenCoordinator.swift`.

2. **Pass Client Credentials into the View**:
   ```swift
   // Inside your coordinator setup:
   let accessToken = parameters.userSession.clientProxy.client.accessToken()
   let homeserver = parameters.userSession.clientProxy.homeserverURL.absoluteString

   // Create the view:
   let assistantView = AssistantTabView(
       accessToken: accessToken,
       homeserverURL: homeserver,
       botRoomID: parameters.userSession.clientProxy.directRoomID(with: "@bot:yourdomain.com")
   )
   ```

3. **Add the Tab to `TabView`**:
   ```swift
   TabView(selection: $selectedTab) {
       HomeScreen(...)
           .tabItem {
               Label("Chats", systemImage: "bubble.left.and.bubble.right")
           }
           .tag(Tab.chats)

       assistantView
           .tabItem {
               Label("Assistant", systemImage: "sparkles")
           }
           .tag(Tab.assistant)

       SettingsScreen(...)
           .tabItem {
               Label("Settings", systemImage: "gearshape")
           }
           .tag(Tab.settings)
   }
   ```

---

## 🔔 5. Why You Don't Need an APNs Server for Reminders

When you create a reminder from the tab:
```swift
try await BotClient.shared.createReminder(
    message: "Meeting in 10 minutes",
    seconds: 600,
    roomID: "!dm_room_with_bot:mbm.one"
)
```

1. When the reminder triggers 10 minutes later, the bot's scheduler executes [`deliver_to_matrix`](backend/src/modules/reminder/scheduler.rs).
2. The bot sends a real Matrix message:
   `⏰ REMINDER for Evan! Meeting in 10 minutes` into that room.
3. The Matrix Homeserver notices a new message in the user's room and triggers its standard Sygnal push notification gateway.
4. **Element X iOS receives the APNs push notification immediately**, plays the sound, and displays the banner on iOS.
5. **No custom Apple Developer push certificates or third-party notification services are needed.**

---

## ⚡ 6. Real-Time Streaming via Server-Sent Events (SSE)

If you want the tab UI to update live when items change:

```swift
public func startLiveEventStream() -> AsyncThrowingStream<String, Error> {
    AsyncThrowingStream { continuation in
        let task = Task {
            guard let sessionToken else {
                continuation.finish(throwing: URLError(.userAuthenticationRequired))
                return
            }

            var request = URLRequest(url: baseURL.appendingPathComponent("events"))
            request.setValue("Bearer \(sessionToken)", forHTTPHeaderField: "Authorization")

            do {
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let httpRes = response as? HTTPURLResponse, httpRes.statusCode == 200 else {
                    continuation.finish(throwing: URLError(.badServerResponse))
                    return
                }

                for try await line in bytes.lines {
                    if line.hasPrefix("data: ") {
                        let json = String(line.dropFirst(6))
                        continuation.yield(json)
                    }
                }
                continuation.finish()
            } catch {
                continuation.finish(throwing: error)
            }
        }

        continuation.onTermination = { _ in
            task.cancel()
        }
    }
}
```

---

## 🔍 7. Troubleshooting & FAQs

### Q: Does the bot backend reject requests without `Origin` headers from iOS?
**No.** We updated [`backend/src/api/auth/extractor.rs`](backend/src/api/auth/extractor.rs). When a request carries `Authorization: Bearer <session_token>`, the browser CSRF `Origin` check is automatically bypassed.

### Q: What happens if the user logs out of Element X?
`client.session.accessToken` is discarded by Element X. The next time the app opens, the tab will re-authenticate with the newly signed-in user's token.

### Q: Can a user access data created before installing the new tab?
**Yes.** The backend resolves accounts via `AccountRepository::resolve_or_provision(&pool, &user_id)`. If the user already chatted with the bot in Matrix, their existing todos, notes, bookmarks, and reminders will automatically load in the tab.
