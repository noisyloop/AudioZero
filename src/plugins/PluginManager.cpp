#include "PluginManager.h"
#include "../dsp/BuiltinEffects.h"

namespace az
{

PluginManager::PluginManager()
{
    formats.addDefaultFormats();
    loadList();
}

PluginManager::~PluginManager()
{
    stopTimer();
}

juce::File PluginManager::settingsFolder()
{
    auto folder = juce::File::getSpecialLocation(juce::File::userApplicationDataDirectory)
#if JUCE_MAC
                      .getChildFile("Application Support")
#endif
                      .getChildFile("NOISYLOOP").getChildFile("AudioZero");
    folder.createDirectory();
    return folder;
}

bool PluginManager::isPluginFile(const juce::File& file)
{
    return file.hasFileExtension("vst3");
}

void PluginManager::loadList()
{
    if (auto xml = juce::XmlDocument::parse(settingsFolder().getChildFile("plugins.xml")))
        knownPlugins.recreateFromXml(*xml);
    if (auto xml = juce::XmlDocument::parse(settingsFolder().getChildFile("plugin-folders.xml")))
        for (auto* e : xml->getChildIterator())
            extraFolders.addIfNotAlreadyThere(e->getStringAttribute("path"));
}

void PluginManager::saveList()
{
    if (auto xml = knownPlugins.createXml())
        xml->writeTo(settingsFolder().getChildFile("plugins.xml"));
    juce::XmlElement folders("FOLDERS");
    for (auto& f : extraFolders)
        folders.createNewChildElement("FOLDER")->setAttribute("path", f);
    folders.writeTo(settingsFolder().getChildFile("plugin-folders.xml"));
}

void PluginManager::startScan(bool rescanAll)
{
    juce::AudioPluginFormat* vst3 = nullptr;
    for (auto* format : formats.getFormats())
        if (format->getName() == "VST3")
            vst3 = format;
    if (vst3 == nullptr || scanner != nullptr)
        return;

    auto paths = vst3->getDefaultLocationsToSearch();
    for (auto& f : extraFolders)
        paths.add(juce::File(f));
    if (rescanAll)
        knownPlugins.clear();

    // A plugin that crashes the scan is recorded here and skipped next time.
    const auto deadMansPedal = settingsFolder().getChildFile("scan-in-progress.txt");
    scanner = std::make_unique<juce::PluginDirectoryScanner>(knownPlugins, *vst3, paths, true, deadMansPedal, false);
    status = "Scanning plugins...";
    sendChangeMessage();
    startTimer(5);
}

void PluginManager::timerCallback()
{
    if (scanner == nullptr)
    {
        stopTimer();
        return;
    }
    juce::String name;
    const bool more = scanner->scanNextFile(true, name);
    status = "Scanning: " + name;
    if (! more)
    {
        stopTimer();
        scanner.reset();
        knownPlugins.sort(juce::KnownPluginList::sortAlphabetically, true);
        status = juce::String(knownPlugins.getNumTypes()) + " plugins found";
        saveList();
    }
    sendChangeMessage();
}

const juce::PluginDescription* PluginManager::findByIdentifier(const juce::String& identifier) const
{
    auto& self = const_cast<PluginManager&>(*this);
    const auto types = knownPlugins.getTypes();
    self.cache.assign(types.begin(), types.end());
    for (auto& d : self.cache)
        if (d.createIdentifierString() == identifier)
            return &d;
    return nullptr;
}

std::unique_ptr<Device> PluginManager::createBuiltin(const juce::String& id) const
{
    auto processor = Builtins::create(id);
    if (processor == nullptr)
        return nullptr;
    auto device = std::make_unique<Device>();
    device->processor = std::move(processor);
    device->builtinId = id;
    return device;
}

std::unique_ptr<Device> PluginManager::createPlugin(const juce::PluginDescription& description, double sampleRate, int blockSize, juce::String& error)
{
    auto instance = formats.createPluginInstance(description, sampleRate, blockSize, error);
    if (instance == nullptr)
        return nullptr;
    auto device = std::make_unique<Device>();
    device->processor = std::move(instance);
    device->description = description;
    return device;
}

std::unique_ptr<Device> PluginManager::createFromFile(const juce::File& file, double sampleRate, int blockSize, juce::String& error)
{
    for (auto* format : formats.getFormats())
    {
        if (! format->fileMightContainThisPluginType(file.getFullPathName()))
            continue;
        juce::OwnedArray<juce::PluginDescription> found;
        format->findAllTypesForFile(found, file.getFullPathName());
        if (found.isEmpty())
            continue;
        for (auto* d : found)
            knownPlugins.addType(*d);
        saveList();
        sendChangeMessage();
        return createPlugin(*found[0], sampleRate, blockSize, error);
    }
    error = "No VST3 plugin found in " + file.getFileName();
    return nullptr;
}

} // namespace az
