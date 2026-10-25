#include "ui/MainComponent.h"
#include "BinaryData.h"

namespace az
{

class MainWindow final : public juce::DocumentWindow
{
public:
    MainWindow(const juce::File& startupFile, bool forceDemo)
        : DocumentWindow("AudioZero", theme::bg0, DocumentWindow::allButtons)
    {
        setUsingNativeTitleBar(true);
        content = new MainComponent(startupFile, forceDemo);
        setContentOwned(content, true);
#if JUCE_MAC
        juce::MenuBarModel::setMacMainMenu(content);
#else
        setMenuBar(content, 24);
        setIcon(juce::ImageCache::getFromMemory(BinaryData::icon_png, BinaryData::icon_pngSize));
#endif
        content->onTitleChanged = [this] { setName(content->windowTitle()); };
        setName(content->windowTitle());
        setResizable(true, true);
        setResizeLimits(1000, 640, 10000, 10000);
        auto area = juce::Desktop::getInstance().getDisplays().getPrimaryDisplay() != nullptr
                        ? juce::Desktop::getInstance().getDisplays().getPrimaryDisplay()->userArea
                        : juce::Rectangle<int>(0, 0, 1600, 1000);
        centreWithSize(std::min(1600, area.getWidth() - 40), std::min(1000, area.getHeight() - 40));
        setVisible(true);
    }

    ~MainWindow() override
    {
#if ! JUCE_MAC
        setMenuBar(nullptr);
#endif
    }

    void closeButtonPressed() override { juce::JUCEApplication::getInstance()->systemRequestedQuit(); }
    MainComponent* content = nullptr;
};

// `AudioZero --screenshot out.png [--tab devices|pianoroll|mixer]` renders the real UI with
// the demo song to a PNG (used for the README) and quits.
class Screenshotter final : private juce::Timer
{
public:
    Screenshotter(MainWindow& w, juce::File out, juce::String tabName) : window(w), file(std::move(out)), tab(std::move(tabName))
    {
        auto& c = *window.content;
        auto& s = c.session;
        c.setSize(1680, 1050);
        s.engine.setPlayhead(38.0);
        auto findTrack = [&s](const juce::String& name) -> std::shared_ptr<Track> {
            for (auto& t : s.project.tracks)
                if (t->data.name == name)
                    return t;
            return nullptr;
        };
        if (tab == "pianoroll")
        {
            if (auto t = findTrack("Bell Hook"))
                s.editClip(t->data.clips.front().id);
        }
        else if (auto t = findTrack("Chords"))
        {
            s.selectClip(t->data.clips.back().id);
            c.showTab(tab == "mixer" ? MainComponent::Tab::mixer : MainComponent::Tab::devices);
        }
        startTimer(1500);
    }

private:
    void timerCallback() override
    {
        stopTimer();
        auto image = window.content->createComponentSnapshot(window.content->getLocalBounds(), true, 1.0f);
        file.deleteFile();
        juce::FileOutputStream out(file);
        juce::PNGImageFormat().writeImageToStream(image, out);
        out.flush();
        std::printf("Screenshot written to %s\n", file.getFullPathName().toRawUTF8());
        juce::JUCEApplication::getInstance()->quit();
    }

    MainWindow& window;
    juce::File file;
    juce::String tab;
};

class Application final : public juce::JUCEApplication
{
public:
    const juce::String getApplicationName() override { return "AudioZero"; }
    const juce::String getApplicationVersion() override { return "0.1.0"; }
    bool moreThanOneInstanceAllowed() override { return true; }

    void initialise(const juce::String& commandLine) override
    {
        juce::LookAndFeel::setDefaultLookAndFeel(&lookAndFeel);
        auto args = juce::StringArray::fromTokens(commandLine, true);
        args.trim();
        args.removeEmptyStrings();
        for (auto& a : args)
            a = a.unquoted();

        const int shot = args.indexOf("--screenshot");
        juce::File startup;
        for (auto& a : args)
            if (a.endsWith(".azproj"))
                startup = juce::File::getCurrentWorkingDirectory().getChildFile(a);

        window = std::make_unique<MainWindow>(startup, shot >= 0);
        if (shot >= 0 && shot + 1 < args.size())
        {
            const int tabIndex = args.indexOf("--tab");
            screenshotter = std::make_unique<Screenshotter>(*window, juce::File::getCurrentWorkingDirectory().getChildFile(args[shot + 1]),
                                                            tabIndex >= 0 && tabIndex + 1 < args.size() ? args[tabIndex + 1] : juce::String("devices"));
        }
    }

    void shutdown() override
    {
        screenshotter.reset();
        window.reset();
        juce::LookAndFeel::setDefaultLookAndFeel(nullptr);
    }

    void systemRequestedQuit() override
    {
        if (window == nullptr || screenshotter != nullptr)
        {
            quit();
            return;
        }
        window->content->requestQuit([] { juce::JUCEApplication::getInstance()->quit(); });
    }

    void anotherInstanceStarted(const juce::String&) override {}

private:
    Theme lookAndFeel;
    std::unique_ptr<MainWindow> window;
    std::unique_ptr<Screenshotter> screenshotter;
};

} // namespace az

START_JUCE_APPLICATION(az::Application)
