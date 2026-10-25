#pragma once
#include "Session.h"
#include "Theme.h"

namespace az
{

// Left-hand browser: built-in devices, Operator presets, scanned VST3 plugins and sample folders.
// Everything can be dragged onto the arrangement / device chain, or double-clicked to add.
class Browser final : public juce::Component, private juce::ListBoxModel, private juce::ChangeListener
{
public:
    explicit Browser(Session&);
    ~Browser() override;
    void paint(juce::Graphics&) override;
    void resized() override;
    void rebuild();
    void addSampleFolder(const juce::File& folder);
    juce::StringArray sampleFolders() const { return folders; }

private:
    struct Row
    {
        juce::String label, detail, item; // item empty => category header
        juce::String category;
        juce::Colour colour;
    };

    int getNumRows() override { return static_cast<int>(rows.size()); }
    void paintListBoxItem(int row, juce::Graphics&, int width, int height, bool selected) override;
    void listBoxItemClicked(int row, const juce::MouseEvent&) override;
    void listBoxItemDoubleClicked(int row, const juce::MouseEvent&) override;
    juce::var getDragSourceDescription(const juce::SparseSet<int>& rows) override;
    juce::String getTooltipForRow(int row) override;
    void changeListenerCallback(juce::ChangeBroadcaster*) override;
    void showAddMenu();
    void saveFolders();

    Session& session;
    juce::TextEditor search;
    juce::ListBox list { "Browser", this };
    juce::TextButton scanButton { "Scan Plugins" }, addButton { "Add Folder..." };
    juce::Label statusLabel;
    std::vector<Row> rows;
    std::set<juce::String> collapsed;
    juce::StringArray folders;
    std::map<juce::String, juce::Array<juce::File>> folderFiles;
    std::unique_ptr<juce::FileChooser> chooser;
};

} // namespace az
