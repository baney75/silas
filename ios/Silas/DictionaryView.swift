import SilasCore
import SwiftUI

/// Your words, corrections, suggestions waiting for a yes, and the fillers Silas drops. Changes
/// here sync to your other devices when sync is on.
struct DictionaryView: View {
    @Bindable var model: DictationModel
    @Environment(\.dismiss) private var dismiss
    @State private var newTerm = ""
    @State private var newFrom = ""
    @State private var newTo = ""
    @State private var newFiller = ""

    private var dict: PersonalDictionary { model.dictionary }

    var body: some View {
        List {
            if !dict.pending.isEmpty {
                Section {
                    ForEach(dict.pending, id: \.from) { p in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text("\(p.from) → \(p.to)").foregroundStyle(Theme.text)
                                Text(p.count == 1 ? "Seen once" : "Seen \(p.count) times")
                                    .font(.system(size: 13)).foregroundStyle(Theme.text3)
                            }
                            Spacer()
                            Button("Accept") { model.acceptPending(p.from) }
                                .buttonStyle(.borderless)
                                .foregroundStyle(Theme.goldInk)
                            Button {
                                model.dismissPending(p.from)
                            } label: {
                                Image(systemName: "xmark").foregroundStyle(Theme.text3)
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel("Dismiss")
                        }
                    }
                } header: {
                    Text("Suggestions")
                } footer: {
                    Text("Fixes Silas noticed in your edits. Accept one and Silas makes it every time.")
                }
            }

            Section {
                HStack {
                    TextField("heard", text: $newFrom)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Image(systemName: "arrow.right").foregroundStyle(Theme.text3)
                    TextField("write", text: $newTo)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(addCorrection)
                    Button("Add", action: addCorrection)
                        .disabled(newFrom.isEmpty || newTo.isEmpty)
                }
                ForEach(dict.corrections, id: \.from) { c in
                    HStack {
                        Text(c.from).foregroundStyle(Theme.text2)
                        Image(systemName: "arrow.right").font(.system(size: 12)).foregroundStyle(Theme.text3)
                        Text(c.to).foregroundStyle(Theme.text)
                        Spacer()
                        if c.count > 0 {
                            Text("\(c.count)×").font(.system(size: 13)).foregroundStyle(Theme.text3)
                        }
                    }
                    .swipeActions { delete { model.removeCorrection(c.from) } }
                }
            } header: {
                Text("Corrections")
            } footer: {
                Text("When Silas hears the left side, it writes the right side.")
            }

            Section {
                HStack {
                    TextField("A name or word, spelled your way", text: $newTerm)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(addTerm)
                    Button("Add", action: addTerm).disabled(newTerm.isEmpty)
                }
                ForEach(dict.terms, id: \.text) { t in
                    Text(t.text)
                        .foregroundStyle(Theme.text)
                        .swipeActions { delete { model.removeTerm(t.text) } }
                }
            } header: {
                Text("Your words")
            } footer: {
                Text("Silas snaps close-sounding words to these spellings.")
            }

            Section {
                HStack {
                    TextField("A filler to drop", text: $newFiller)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(addFiller)
                    Button("Add", action: addFiller).disabled(newFiller.isEmpty)
                }
                ForEach(dict.fillers, id: \.self) { f in
                    Text(f)
                        .foregroundStyle(Theme.text2)
                        .swipeActions { delete { model.removeFiller(f) } }
                }
            } header: {
                Text("Fillers")
            }
        }
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
        .listRowBackground(Theme.surface)
        .navigationTitle("Dictionary")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
    }

    private func delete(_ action: @escaping () -> Void) -> some View {
        Button(role: .destructive, action: action) {
            Label("Delete", systemImage: "trash")
        }
    }

    private func addTerm() {
        if model.addTerm(newTerm) { newTerm = "" }
    }

    private func addCorrection() {
        if model.addCorrection(newFrom, newTo) {
            newFrom = ""
            newTo = ""
        }
    }

    private func addFiller() {
        model.addFiller(newFiller)
        newFiller = ""
    }
}
